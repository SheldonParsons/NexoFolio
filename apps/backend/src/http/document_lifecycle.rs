//! All environment-independent lifecycle operations use the shared project grant.
use super::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::*;
use std::sync::Arc;
#[derive(Clone)]
struct LifecycleHttp {
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    reader: Arc<dyn KnowledgeReader>,
    writer: Arc<dyn DocumentLifecycleWriter>,
}
pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    reader: Arc<dyn KnowledgeReader>,
    writer: Arc<dyn DocumentLifecycleWriter>,
) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project}/document-lifecycle",
            get(read).post(change),
        )
        .route_layer(middleware::from_fn_with_state(
            sessions.clone(),
            session_auth,
        ))
        .with_state(LifecycleHttp {
            sessions,
            projects,
            reader,
            writer,
        })
        .layer(middleware::from_fn(no_store))
}
fn error(e: KnowledgeError) -> Response {
    let (status, code) = match e {
        KnowledgeError::LifecycleStale => (StatusCode::CONFLICT, "LIFECYCLE_STALE"),
        KnowledgeError::WrongProject | KnowledgeError::NoSuchFolder => {
            (StatusCode::NOT_FOUND, "NOT_FOUND")
        }
        KnowledgeError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE"),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_LIFECYCLE"),
    };
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":e.to_string()}})),
    )
        .into_response()
}
async fn authorize(
    s: &LifecycleHttp,
    principal: &SessionPrincipal,
    project: ProjectId,
) -> Result<(), Response> {
    s.projects
        .require_project(principal, project)
        .await
        .map(|_| ())
        .map_err(|e| AccessError::from(e).into_response())
}
async fn read(
    State(s): State<LifecycleHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<DocumentLifecycle>, Response> {
    authorize(&s, &p, project).await?;
    s.reader.lifecycle(project).await.map(Json).map_err(error)
}
async fn change(
    State(s): State<LifecycleHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(change): Json<LifecycleChange>,
) -> Result<Json<DocumentLifecycle>, Response> {
    authorize(&s, &p, project).await?;
    let actor = super::curation_actor::from_session(s.sessions.as_ref(), &p)
        .await
        .map_err(|e| AccessError::from(e).into_response())?;
    s.writer
        .change_lifecycle(project, change, actor)
        .await
        .map(Json)
        .map_err(error)
}
