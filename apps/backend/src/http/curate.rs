//! Curate API: trigger describe and organize passes that write Knowledge Commands.

use crate::http::access::{session_auth, AccessError, no_store};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware, Extension, Json, Router,
    response::{IntoResponse, Response},
    routing::post,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::{KnowledgeError, KnowledgeWriter, RoundState};
use nexofolio_curate::{CompletionError, Completions, Describe, Organize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
struct CurateHttp {
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeWriter>,
    endpoints: Arc<dyn nexofolio_contracts::endpoint::EndpointReader>,
    describe: Arc<Describe<Arc<dyn Completions>>>,
    organize: Arc<Organize<Arc<dyn Completions>>>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeWriter>,
    endpoints: Arc<dyn nexofolio_contracts::endpoint::EndpointReader>,
    describe: Arc<Describe<Arc<dyn Completions>>>,
    organize: Arc<Organize<Arc<dyn Completions>>>,
) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project_id}/curate/describe/{endpoint_id}",
            post(describe_endpoint),
        )
        .route(
            "/v1/projects/{project_id}/curate/organize",
            post(organize_project),
        )
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(CurateHttp {
            projects,
            knowledge,
            endpoints,
            describe,
            organize,
        })
        .layer(middleware::from_fn(no_store))
}

enum CurateHttpError {
    Access(AccessError),
    NotFound,
    Unavailable,
    Failed(String),
}

impl From<AccessError> for CurateHttpError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<KnowledgeError> for CurateHttpError {
    fn from(error: KnowledgeError) -> Self {
        match error {
            KnowledgeError::Unavailable => Self::Unavailable,
            _ => Self::Failed(error.to_string()),
        }
    }
}

impl From<CompletionError> for CurateHttpError {
    fn from(error: CompletionError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl IntoResponse for CurateHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "接口不存在".to_string(),
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用".to_string(),
            ),
            Self::Failed(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "CURATION_FAILED",
                msg,
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

async fn describe_endpoint(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, endpoint)): Path<(ProjectId, EndpointId)>,
) -> Result<Json<Value>, CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;

    let Some(facts) = s.endpoints.get(endpoint).await.map_err(|_| CurateHttpError::Unavailable)? else {
        return Err(CurateHttpError::NotFound);
    };

    if facts.summary.project_id != project {
        return Err(CurateHttpError::NotFound);
    }

    let command = s.describe.endpoint(&facts).await?;

    if let Some(cmd) = command {
        let round = s.knowledge.begin(project, s.describe.author()).await?;
        match s.knowledge.apply(round, &[cmd]).await {
            Ok(_) => {
                s.knowledge.finish(round, RoundState::Done, Some("描述接口".into())).await?;
                Ok(Json(json!({ "status": "ok" })))
            }
            Err(e) => {
                let _ = s.knowledge.finish(round, RoundState::Failed, Some("描述失败".into())).await;
                Err(e.into())
            }
        }
    } else {
        Ok(Json(json!({ "status": "skipped" })))
    }
}

async fn organize_project(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;

    let summaries = s.endpoints.list(project).await.map_err(|_| CurateHttpError::Unavailable)?;

    let commands = s.organize.project(&summaries).await?;

    if !commands.is_empty() {
        let round = s.knowledge.begin(project, s.organize.author()).await?;
        match s.knowledge.apply(round, &commands).await {
            Ok(_) => {
                s.knowledge
                    .finish(round, RoundState::Done, Some(format!("组织 {} 个接口", summaries.len())))
                    .await?;
                Ok(Json(json!({ "status": "ok", "commands": commands.len() })))
            }
            Err(e) => {
                let _ = s.knowledge.finish(round, RoundState::Failed, Some("组织失败".into())).await;
                Err(e.into())
            }
        }
    } else {
        Ok(Json(json!({ "status": "skipped" })))
    }
}
