//! Review endpoints authorize a project and dispatch knowledge lifecycle operations.
use super::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access_contracts::{ActorNames, ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{
    ApplyObserver, ApplyProgress, ApplyStage, CandidateActor, CandidateEntryKind,
    CandidateReviewQuery, CandidateSummary, KnowledgeCandidates, KnowledgeError, ReviewPart,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct ReviewHttp {
    sessions: Arc<dyn Sessions>,
    actors: Arc<dyn ActorNames>,
    projects: Arc<dyn ProjectAccess>,
    candidates: Arc<dyn KnowledgeCandidates>,
    observations: super::curation_apply::ApplyObservations,
}
pub(crate) fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    candidates: Arc<dyn KnowledgeCandidates>,
    actors: Arc<dyn ActorNames>,
) -> Router {
    Router::new()
        .route("/v1/projects/{project}/curate/candidates", get(list))
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}",
            get(detail),
        )
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}/review",
            get(review),
        )
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}/apply-status",
            get(apply_status),
        )
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}/apply",
            post(apply),
        )
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}/operations",
            get(operations),
        )
        .route(
            "/v1/projects/{project}/curate/candidates/{candidate}/discard",
            post(discard),
        )
        .route_layer(middleware::from_fn_with_state(
            sessions.clone(),
            session_auth,
        ))
        .with_state(ReviewHttp {
            sessions,
            actors,
            projects,
            candidates,
            observations: Default::default(),
        })
        .layer(middleware::from_fn(no_store))
}
enum ReviewError {
    Changed,
    Access(AccessError),
    Knowledge(KnowledgeError),
}
impl From<KnowledgeError> for ReviewError {
    fn from(e: KnowledgeError) -> Self {
        Self::Knowledge(e)
    }
}
impl From<AccessError> for ReviewError {
    fn from(e: AccessError) -> Self {
        Self::Access(e)
    }
}
impl IntoResponse for ReviewError {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Changed => return (StatusCode::CONFLICT,Json(json!({"error":{"code":"REVIEW_CHANGED","message":"候选内容已更新，请重新读取当前范围"}}))).into_response(),
            Self::Access(e) => return e.into_response(),
            Self::Knowledge(e) => e,
        };
        let (status, code, message) = match error {
            KnowledgeError::HistoryExpired => (
                StatusCode::GONE,
                "HISTORY_EXPIRED",
                "历史已精简，仅保留摘要；详情、对比与回滚已到期",
            ),
            KnowledgeError::NoSuchCandidate => {
                (StatusCode::NOT_FOUND, "NOT_FOUND", "整理结果不存在")
            }
            KnowledgeError::LifecycleStale => (
                StatusCode::CONFLICT,
                "LIFECYCLE_STALE",
                "废弃状态已变化，此结果保留但不能应用；请开始新一轮整理",
            ),
            KnowledgeError::CandidateStale => (
                StatusCode::CONFLICT,
                "CANDIDATE_STALE",
                "生成后现行文档已有变化，请重新整理；此候选结果仍保留",
            ),
            KnowledgeError::CandidateClosed => (
                StatusCode::CONFLICT,
                "CANDIDATE_CLOSED",
                "此结果当前不能执行该操作",
            ),
            KnowledgeError::ProtectedKnowledge => (
                StatusCode::CONFLICT,
                "PROTECTED_KNOWLEDGE",
                "此变更会覆盖人工内容，未应用任何变更",
            ),
            KnowledgeError::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "暂时无法保存决定，请稍后重新读取结果状态",
            ),
            _ => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INVALID_CANDIDATE",
                "候选结果未通过知识校验，未应用任何变更",
            ),
        };
        crate::audit::http::problem(
            (
                status,
                Json(json!({"error":{"code":code,"message":message}})),
            )
                .into_response(),
            code,
        )
    }
}
async fn authorize(
    s: &ReviewHttp,
    principal: &SessionPrincipal,
    project: ProjectId,
) -> Result<(), ReviewError> {
    s.projects
        .require_project(principal, project)
        .await
        .map_err(AccessError::from)?;
    Ok(())
}
#[derive(serde::Deserialize)]
struct ListOptions {
    #[serde(default)]
    review_entry: bool,
    #[serde(default)]
    panel_entry: bool,
    selected_id: Option<uuid::Uuid>,
}
async fn list(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(options): Query<ListOptions>,
) -> Result<Json<Vec<CandidateSummary>>, ReviewError> {
    authorize(&s, &principal, project).await?;
    Ok(Json(if options.panel_entry {
        s.candidates
            .entry_candidates(project, CandidateEntryKind::Panel, options.selected_id)
            .await?
    } else if options.review_entry {
        s.candidates
            .entry_candidates(project, CandidateEntryKind::Review, None)
            .await?
    } else {
        s.candidates.candidates(project, 30).await?
    }))
}
async fn detail(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<Response, ReviewError> {
    authorize(&s, &principal, project).await?;
    let start = std::time::Instant::now();
    let value = s.candidates.candidate(project, id).await?;
    let read = start.elapsed();
    let response = Json(value).into_response();
    tracing::info!(
        read_ms = read.as_millis() as u64,
        serialize_ms = (start.elapsed() - read).as_millis() as u64,
        "candidate detail timing"
    );
    Ok(response)
}
async fn review(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
    Query(query): Query<CandidateReviewQuery>,
) -> Result<Response, ReviewError> {
    authorize(&s, &principal, project).await?;
    let start = std::time::Instant::now();
    let value = s
        .candidates
        .review_candidate(project, id, &query)
        .await
        .map_err(|e| {
            if matches!(e, KnowledgeError::CandidateStale) {
                ReviewError::Changed
            } else {
                e.into()
            }
        })?;
    let read = start.elapsed();
    let response = Json(value).into_response();
    tracing::info!(
        read_ms = read.as_millis() as u64,
        serialize_ms = (start.elapsed() - read).as_millis() as u64,
        "candidate review timing"
    );
    Ok(response)
}
#[derive(serde::Deserialize)]
struct ApplyQuery {
    operation: Option<uuid::Uuid>,
}
async fn apply_status(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
    Query(query): Query<ApplyQuery>,
) -> Result<Json<Value>, ReviewError> {
    authorize(&s, &principal, project).await?;
    let state = s
        .candidates
        .review_candidate(
            project,
            id,
            &CandidateReviewQuery {
                part: ReviewPart::Status,
                ..Default::default()
            },
        )
        .await?;
    let progress = query
        .operation
        .and_then(|op| s.observations.get(project, id, op));
    Ok(Json(
        json!({"candidate":state,"operation":query.operation,"progress":progress}),
    ))
}
async fn apply(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
    Query(query): Query<ApplyQuery>,
) -> Result<Json<Value>, ReviewError> {
    authorize(&s, &principal, project).await?;
    let actor = super::curation_actor::from_session(s.sessions.as_ref(), &principal)
        .await
        .map_err(AccessError::from)?;
    let operation = query.operation.unwrap_or_else(uuid::Uuid::new_v4);
    let observer = s
        .observations
        .start(project, id, operation)
        .ok_or(KnowledgeError::CandidateClosed)?;
    let result = s
        .candidates
        .apply_candidate_observed(project, id, actor, &observer)
        .await;
    match result {
        Ok(rounds) => {
            observer.update(ApplyProgress {
                stage: ApplyStage::Committed,
                processed: 0,
                total: 0,
            });
            Ok(Json(
                json!({"status":"applied","rounds":rounds,"operation":operation}),
            ))
        }
        Err(error) => {
            // A lost COMMIT acknowledgement is uncertain, not proof of rollback.
            observer.update(ApplyProgress {
                stage: if matches!(error, KnowledgeError::Unavailable) {
                    ApplyStage::Unknown
                } else {
                    ApplyStage::Failed
                },
                processed: 0,
                total: 0,
            });
            Err(error.into())
        }
    }
}
async fn discard(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<Json<Value>, ReviewError> {
    authorize(&s, &principal, project).await?;
    s.candidates
        .discard_candidate_as(
            project,
            id,
            super::curation_actor::from_session(s.sessions.as_ref(), &principal)
                .await
                .map_err(AccessError::from)?,
        )
        .await?;
    Ok(Json(json!({"status":"discarded"})))
}
async fn operations(
    State(s): State<ReviewHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<Json<Value>, ReviewError> {
    authorize(&s, &principal, project).await?;
    let candidate = s.candidates.candidate_operations(project, id).await?;
    let events = super::curation_operations::events(&candidate);
    let missing: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.actor {
            CandidateActor::User {
                id,
                display_name: None,
            } => Some(*id),
            _ => None,
        })
        .collect();
    let names = s.actors.actor_names(&missing).await;
    let unavailable = names.is_err();
    let events = super::curation_operations::resolve(events, &names.unwrap_or_default());
    Ok(Json(
        json!({ "events": events, "names_unavailable": unavailable }),
    ))
}
