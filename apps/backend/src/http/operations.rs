//! Private read transport. All SQL and metric IO remain in their owning read adapters.
use super::{
    access::{AccessError, no_store},
    management::administrator,
};
use crate::{audit::console, operations::Monitor};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::get,
};
use nexofolio_access_contracts::SessionPrincipal;
use nexofolio_common::{Error, ProjectId};
use nexofolio_contracts::knowledge::TaskState;
use nexofolio_knowledge_contracts::operations::{OperationsReader, TaskCursor, TaskFilter};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Services {
    pub monitor: Monitor,
    pub tasks: Arc<dyn OperationsReader>,
    pub diagnostics: console::Services,
}
pub(crate) fn routes(s: Services) -> Router {
    Router::new()
        .route("/v1/operations/summary", get(summary))
        .route("/v1/operations/tasks", get(tasks))
        .route("/v1/operations/tasks/{task}/diagnostics", get(diagnostics))
        .route_layer(middleware::from_fn_with_state(
            s.diagnostics.sessions.clone(),
            administrator,
        ))
        .with_state(s)
        .layer(middleware::from_fn(no_store))
}
fn unavailable() -> AccessError {
    AccessError(Error::Unavailable {
        component: "operations",
    })
}
async fn summary(State(s): State<Services>) -> Result<Json<Value>, AccessError> {
    Ok(Json(
        json!({"observed_at": chrono::Utc::now(), "snapshot": s.monitor.summary().ok_or_else(unavailable)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    project: Option<ProjectId>,
    state: Option<TaskState>,
    before_at: Option<chrono::DateTime<chrono::Utc>>,
    before_id: Option<uuid::Uuid>,
    limit: Option<u32>,
}
async fn tasks(
    State(s): State<Services>,
    Extension(principal): Extension<SessionPrincipal>,
    Query(q): Query<Filter>,
) -> Result<Json<Value>, AccessError> {
    let before = match (q.before_at, q.before_id) {
        (Some(created_at), Some(id)) => Some(TaskCursor { created_at, id }),
        (None, None) => None,
        _ => return Err(AccessError(Error::invalid("incomplete task cursor"))),
    };
    let limit = q.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(AccessError(Error::invalid("limit must be 1..50")));
    }
    let page = s
        .tasks
        .operations_tasks(&TaskFilter {
            project: q.project,
            state: q.state,
            before,
            limit,
        })
        .await
        .map_err(|_| unavailable())?;
    let mut projects = std::collections::BTreeMap::new();
    for project in page
        .items
        .iter()
        .map(|t| t.project_id)
        .collect::<std::collections::HashSet<_>>()
    {
        if let Ok(card) = s
            .diagnostics
            .projects
            .require_project(&principal, project)
            .await
        {
            projects.insert(project.to_string(), card.name);
        }
    }
    Ok(Json(
        json!({"page": page, "projects": projects, "source": "knowledge.curation_tasks + bounded write-time operations_checkpoint; old summaries may be uncollected; batches are on-demand only", "scope": "database_global; filtered by project and state", "stale_after_seconds": 20, "limit": limit}),
    ))
}
async fn diagnostics(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Value>, AccessError> {
    let project = s
        .tasks
        .task_project(id)
        .await
        .map_err(|_| unavailable())?
        .ok_or(AccessError(Error::NotFound))?;
    // Reuse the complete existing project authorization and schema-2 export path.
    console::task_diagnostic(State(s.diagnostics), Extension(p), Path((project, id))).await
}
