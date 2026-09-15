use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::{get, post},
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_application::{
    KnowledgeActivationStore, KnowledgePublicationService, MaintenanceStore,
};
use nexofolio_contracts::*;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone)]
pub struct MaintenanceHttp {
    pub access: Arc<dyn PlatformAccess>,
    pub store: Arc<dyn MaintenanceStore>,
    pub knowledge: Arc<dyn KnowledgeActivationStore>,
    pub publication: Arc<KnowledgePublicationService>,
    pub model_configured: bool,
}
use super::Page;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Env {
    environment_id: EnvironmentId,
}
pub fn routes(s: MaintenanceHttp) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project}/maintenance-runs",
            post(start).get(list),
        )
        .route("/v1/projects/{project}/maintenance-runs/{run}", get(detail))
        .route(
            "/v1/projects/{project}/maintenance-runs/{run}/snapshot",
            get(snapshot),
        )
        .route(
            "/v1/projects/{project}/maintenance-runs/{run}/checkpoints",
            get(checkpoints),
        )
        .route(
            "/v1/projects/{project}/maintenance-runs/{run}/publish",
            post(publish),
        )
        .route("/v1/projects/{project}/knowledge/versions", get(versions))
        .route("/v1/projects/{project}/knowledge/restore", post(restore))
        .route(
            "/v1/projects/{project}/interfaces/{interface}/knowledge",
            get(interface),
        )
        .route_layer(middleware::from_fn_with_state(
            s.access.clone(),
            session_auth,
        ))
        .with_state(s)
        .layer(middleware::from_fn(super::access::no_store))
}
async fn start(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(q): Json<StartMaintenance>,
) -> std::result::Result<Json<MaintenanceRun>, AccessError> {
    s.access.require_project(&p, project).await?;
    if !s.model_configured {
        return Err(Error::NotConfigured {
            capability: "maintenance_model",
        }
        .into());
    }
    Ok(Json(s.store.start(p.user_id, project, &q).await?))
}
async fn list(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<Page>,
) -> std::result::Result<Json<MaintenancePage>, AccessError> {
    Ok(Json(
        s.store.list(p.user_id, project, q.page, q.limit).await?,
    ))
}
async fn detail(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, run)): Path<(ProjectId, Uuid)>,
) -> std::result::Result<Json<MaintenanceRun>, AccessError> {
    Ok(Json(s.store.get(p.user_id, project, run).await?))
}
async fn snapshot(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, run)): Path<(ProjectId, Uuid)>,
) -> std::result::Result<Json<KnowledgeSnapshot>, AccessError> {
    Ok(Json(s.store.snapshot(p.user_id, project, run).await?))
}
async fn checkpoints(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, run)): Path<(ProjectId, Uuid)>,
    Query(q): Query<Page>,
) -> std::result::Result<Json<MaintenanceCheckpointPage>, AccessError> {
    Ok(Json(
        s.store
            .checkpoints(p.user_id, project, run, q.page, q.limit)
            .await?,
    ))
}
async fn publish(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, run)): Path<(ProjectId, Uuid)>,
    Json(q): Json<PublishKnowledge>,
) -> std::result::Result<Json<KnowledgeActivation>, AccessError> {
    Ok(Json(
        s.publication.publish(p.user_id, project, run, &q).await?,
    ))
}
async fn versions(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<Page>,
) -> std::result::Result<Json<KnowledgeVersionPage>, AccessError> {
    Ok(Json(
        s.knowledge
            .knowledge_versions(p.user_id, project, q.page, q.limit)
            .await?,
    ))
}
async fn restore(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(q): Json<RestoreKnowledge>,
) -> std::result::Result<Json<KnowledgeActivation>, AccessError> {
    Ok(Json(s.publication.restore(p.user_id, project, &q).await?))
}
async fn interface(
    State(s): State<MaintenanceHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, interface)): Path<(ProjectId, InterfaceId)>,
    Query(q): Query<Env>,
) -> std::result::Result<Json<InterfaceKnowledge>, AccessError> {
    Ok(Json(
        s.knowledge
            .interface_knowledge(p.user_id, project, interface, q.environment_id)
            .await?,
    ))
}
