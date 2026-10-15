//! Private transport: cached reads, bounded sampling requests, explicit maintenance.
use super::{
    access::{AccessError, no_store},
    management::administrator,
};
use crate::{audit::console, operations::storage::StorageMonitor};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    middleware,
    routing::{get, post},
};
use nexofolio_access_contracts::SessionPrincipal;
use nexofolio_common::{Error, ProjectId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
#[derive(Clone)]
pub(crate) struct Services {
    pub monitor: StorageMonitor,
    pub access: console::Services,
}
pub(crate) fn routes(s: Services) -> Router {
    Router::new()
        .route("/v1/projects/{project}/storage", get(read))
        .route("/v1/projects/{project}/storage/refresh", post(refresh))
        .route(
            "/v1/projects/{project}/storage/maintenance",
            post(maintenance),
        )
        .route_layer(middleware::from_fn_with_state(
            s.access.sessions.clone(),
            administrator,
        ))
        .with_state(s)
        .layer(middleware::from_fn(no_store))
}
async fn authorize(
    s: &Services,
    p: &SessionPrincipal,
    project: ProjectId,
) -> Result<(), AccessError> {
    s.access.projects.require_project(p, project).await?;
    Ok(())
}
async fn read(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, AccessError> {
    authorize(&s, &p, project).await?;
    Ok(Json(
        json!({"snapshot":s.monitor.read(project),"execution_allowed":s.monitor.execution_allowed}),
    ))
}
async fn refresh(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, AccessError> {
    authorize(&s, &p, project).await?;
    s.monitor.request(project);
    Ok(Json(json!({"queued":true,"project":project})))
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Action {
    Preview,
    Enable,
    Execute,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    action: Action,
    after: Option<uuid::Uuid>,
}
async fn maintenance(
    State(s): State<Services>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(q): Json<Command>,
) -> Result<Json<Value>, AccessError> {
    authorize(&s, &p, project).await?;
    let execute = q.action != Action::Preview;
    if execute && !s.monitor.execution_allowed {
        return Err(AccessError(Error::invalid("服务器尚未允许执行清理")));
    }
    let _guard = s
        .monitor
        .execution
        .try_lock()
        .map_err(|_| AccessError(Error::invalid("维护正忙，请稍后重新预览")))?;
    let status = tokio::time::timeout(
        Duration::from_secs(1),
        s.monitor.maintenance.retention_status(project),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .ok_or(AccessError(Error::Unavailable {
        component: "storage_state",
    }))?;
    if q.action == Action::Enable && (status.enabled_at.is_some() || q.after.is_some()) {
        return Err(AccessError(Error::invalid("项目已启用，请重新读取状态")));
    }
    if q.action == Action::Execute && status.enabled_at.is_none() {
        return Err(AccessError(Error::invalid(
            "必须先明确启用项目并等待7天保护期",
        )));
    }
    // Authorization is checked again immediately before invoking the locked engine.
    let user = s.access.sessions.me(&p).await?;
    if !nexofolio_access_contracts::can_manage_projects(&user.account, user.enabled) {
        return Err(AccessError(Error::NotFound));
    }
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        s.monitor.maintenance.maintain(project, q.after, execute),
    )
    .await;
    if execute {
        s.monitor.invalidate(project);
    }
    Ok(Json(match result {
        Ok(Ok(report)) => {
            json!({"project":project,"outcome":if execute && report.committed {"committed"} else if execute {"not_committed"} else {"preview"},"report":report,"at":chrono::Utc::now()})
        }
        _ => {
            json!({"project":project,"outcome":if execute {"unknown"} else {"failed"},"report":null,"at":chrono::Utc::now()})
        }
    }))
}
