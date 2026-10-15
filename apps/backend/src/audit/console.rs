//! Private audit queries compose module read contracts. No cross-schema SQL or model calls.
use super::{Audit, diagnostics};
use crate::http::access::{AccessError, authenticate, no_store};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{
    ActorNames, ProjectAccess, SessionPrincipal, Sessions, can_manage_projects,
};
use nexofolio_audit_contracts::{AccessQuery, AuditStore};
use nexofolio_common::{Error, ProjectId};
use nexofolio_contracts::{
    endpoint::EndpointReader,
    knowledge::{KnowledgeCandidates, KnowledgeError, KnowledgeReader, KnowledgeTasks},
};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
#[derive(Clone)]
pub(crate) struct Services {
    pub sessions: Arc<dyn Sessions>,
    pub projects: Arc<dyn ProjectAccess>,
    pub actors: Arc<dyn ActorNames>,
    pub store: Arc<dyn AuditStore>,
    pub candidates: Arc<dyn KnowledgeCandidates>,
    pub tasks: Arc<dyn KnowledgeTasks>,
    pub knowledge: Arc<dyn KnowledgeReader>,
    pub endpoints: Arc<dyn EndpointReader>,
}
async fn manager(State(s): State<Services>, mut request: Request, next: Next) -> Response {
    let headers = request.headers().clone();
    let authorized = async {
        let p = authenticate(s.sessions.as_ref(), &headers).await?;
        let user = s.sessions.me(&p).await?;
        if user.id != p.user_id || !can_manage_projects(&user.account, user.enabled) {
            return Err(Error::NotFound);
        }
        Ok(p)
    }
    .await;
    match authorized {
        Ok(p) => {
            request.extensions_mut().insert(p);
            next.run(request).await
        }
        Err(_) => AccessError(Error::NotFound).into_response(),
    }
}
pub(crate) fn routes(s: Services) -> Router {
    Router::new()
        .route("/v1/audit/access", get(access))
        .route("/v1/audit/curations/{project}", get(curations))
        .route("/v1/audit/curations/{project}/{candidate}", get(diagnostic))
        .route(
            "/v1/audit/curations/{project}/tasks/{task}",
            get(task_diagnostic),
        )
        .route_layer(middleware::from_fn_with_state(s.clone(), manager))
        .with_state(s)
        .layer(middleware::from_fn(no_store))
}
fn knowledge_error(error: KnowledgeError) -> AccessError {
    AccessError(match error {
        KnowledgeError::NoSuchCandidate => Error::NotFound,
        _ => Error::Unavailable {
            component: "curation_diagnostics",
        },
    })
}
async fn names(s: &Services, p: &SessionPrincipal, value: &mut Value) {
    let rows = value["items"].as_array().cloned().unwrap_or_default();
    let ids: HashSet<_> = rows
        .iter()
        .filter_map(|r| r["actor"].as_str())
        .filter_map(|a| a.strip_prefix("user:"))
        .filter_map(|a| a.parse().ok())
        .collect();
    if let Ok(names) = s
        .actors
        .actor_names(&ids.into_iter().collect::<Vec<_>>())
        .await
    {
        value["actors"] = json!(
            names
                .into_iter()
                .map(|a| (format!("user:{}", a.id), a.display_name))
                .collect::<std::collections::BTreeMap<_, _>>()
        );
    }
    let projects: HashSet<ProjectId> = rows
        .iter()
        .filter_map(|r| r["project"].as_str())
        .filter_map(|v| v.parse().ok())
        .collect();
    let mut project_names = json!({});
    let mut resources = json!({});
    for project in projects {
        if let Ok(card) = s.projects.require_project(p, project).await {
            project_names[project.to_string()] = json!(card.name);
            let needed: HashSet<String> = rows
                .iter()
                .filter(|r| r["project"] == project.to_string())
                .filter_map(|r| r["resource"].as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
            if needed.is_empty() {
                continue;
            }
            if let Ok(notes) = s.knowledge.notes(project).await {
                for note in notes {
                    if note.name.is_some() && needed.contains(&note.endpoint.to_string()) {
                        resources[note.endpoint.to_string()] = json!(note.name);
                    }
                }
            }
            if let Ok(endpoints) = s.endpoints.list(project).await {
                for e in endpoints {
                    if needed.contains(&e.id.to_string())
                        && resources.get(e.id.to_string()).is_none()
                    {
                        resources[e.id.to_string()] =
                            json!(format!("{} {}", e.method, e.path_template));
                    }
                }
            }
            if let Ok(catalogue) = s.knowledge.catalogue(project).await {
                for folder in catalogue.folders {
                    if needed.contains(&folder.id.to_string()) {
                        resources[folder.id.to_string()] = json!(folder.name);
                    }
                }
            }
        }
    }
    value["projects"] = project_names;
    value["resources"] = resources;
}
async fn access(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    audit: Option<Extension<Audit>>,
    Query(query): Query<AccessQuery>,
) -> Result<Json<Value>, AccessError> {
    query.validate()?;
    let mut report = s.store.access_report(&query).await?;
    names(&s, &p, &mut report).await;
    if let Some(Extension(audit)) = audit {
        report["health"] = audit.health();
        report["retention"] = json!({"days":audit.settings.limits.access_days,"max_details":audit.settings.limits.events});
    }
    Ok(Json(report))
}
async fn curations(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, AccessError> {
    let card = s.projects.require_project(&p, project).await?;
    let rows = s
        .candidates
        .candidates(project, 100)
        .await
        .map_err(knowledge_error)?;
    let mut items = Vec::new();
    for row in &rows {
        let task = s
            .tasks
            .task_for_candidate(project, row.id)
            .await
            .map_err(knowledge_error)?;
        items.push(diagnostics::summary(row, task.as_ref()));
    }
    for task in s
        .tasks
        .active_tasks(project)
        .await
        .map_err(knowledge_error)?
    {
        if task.candidate_id.is_none() {
            items.push(diagnostics::task_summary(&task));
        }
    }
    Ok(Json(
        json!({"project":{"id":project,"name":card.name},"items":items,"limit":100,"scope":"latest_100","note":"最近 100 轮候选及尚无候选的活动任务；旧轮次可按候选 ID 精确读取，不代表全部历史。"}),
    ))
}
async fn diagnostic(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, candidate)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<Json<Value>, AccessError> {
    let card = s.projects.require_project(&p, project).await?;
    let candidate = s
        .candidates
        .candidate_diagnostics(project, candidate)
        .await
        .map_err(knowledge_error)?;
    let task = s
        .tasks
        .task_for_candidate(project, candidate.summary.id)
        .await
        .map_err(knowledge_error)?;
    let task_id = task.as_ref().map(|t| t.id.to_string());
    let events = s
        .store
        .curation_events(
            &project.to_string(),
            Some(&candidate.summary.id.to_string()),
            task_id.as_deref(),
        )
        .await
        .unwrap_or(json!({"items":[],"available":false,"total":null}));
    Ok(Json(diagnostics::package(
        Some(&candidate),
        task.as_ref(),
        project,
        &card.name,
        events,
    )))
}
pub(crate) async fn task_diagnostic(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<Json<Value>, AccessError> {
    let card = s.projects.require_project(&p, project).await?;
    let task = s.tasks.task(project, id).await.map_err(knowledge_error)?;
    let candidate = if let Some(id) = task.candidate_id {
        Some(
            s.candidates
                .candidate_diagnostics(project, id)
                .await
                .map_err(knowledge_error)?,
        )
    } else {
        None
    };
    let candidate_id = task.candidate_id.map(|id| id.to_string());
    let events = s
        .store
        .curation_events(
            &project.to_string(),
            candidate_id.as_deref(),
            Some(&task.id.to_string()),
        )
        .await
        .unwrap_or(json!({"items":[],"available":false,"total":null}));
    Ok(Json(diagnostics::package(
        candidate.as_ref(),
        Some(&task),
        project,
        &card.name,
        events,
    )))
}
