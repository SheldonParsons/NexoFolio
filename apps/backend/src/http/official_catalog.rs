use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::{get, post},
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_application::CatalogPublicationService;
use nexofolio_contracts::*;
use nexofolio_knowledge::OfficialCatalogReader;
use serde::Deserialize;
use std::sync::Arc;
#[derive(Clone)]
pub struct OfficialCatalogHttp {
    pub access: Arc<dyn PlatformAccess>,
    pub reader: Arc<dyn OfficialCatalogReader>,
    pub publication: Arc<CatalogPublicationService>,
}
use super::Page;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Interfaces {
    directory_id: Option<DirectoryId>,
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
    expected_generation: Option<i64>,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}
pub fn routes(s: OfficialCatalogHttp) -> Router {
    Router::new()
        .route("/v1/projects/{project}/catalog", get(current))
        .route("/v1/projects/{project}/catalog/interfaces", get(interfaces))
        .route("/v1/projects/{project}/catalog/versions", get(versions))
        .route("/v1/projects/{project}/catalog/publish", post(publish))
        .route("/v1/projects/{project}/catalog/restore", post(restore))
        .route_layer(middleware::from_fn_with_state(
            s.access.clone(),
            session_auth,
        ))
        .with_state(s)
        .layer(middleware::from_fn(super::access::no_store))
}
async fn current(
    State(s): State<OfficialCatalogHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> std::result::Result<Json<OfficialCatalog>, AccessError> {
    Ok(Json(s.reader.current(p.user_id, project).await?))
}
async fn interfaces(
    State(s): State<OfficialCatalogHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<Interfaces>,
) -> std::result::Result<Json<OfficialInterfacePage>, AccessError> {
    Ok(Json(
        s.reader
            .interfaces(
                p.user_id,
                project,
                q.directory_id,
                q.page,
                q.limit,
                q.expected_generation,
            )
            .await?,
    ))
}
async fn versions(
    State(s): State<OfficialCatalogHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<Page>,
) -> std::result::Result<Json<CatalogVersionPage>, AccessError> {
    Ok(Json(
        s.reader
            .versions(p.user_id, project, q.page, q.limit)
            .await?,
    ))
}
async fn publish(
    State(s): State<OfficialCatalogHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(q): Json<PublishCatalog>,
) -> std::result::Result<Json<CatalogActivation>, AccessError> {
    Ok(Json(s.publication.publish(p.user_id, project, &q).await?))
}
async fn restore(
    State(s): State<OfficialCatalogHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(q): Json<RestoreCatalog>,
) -> std::result::Result<Json<CatalogActivation>, AccessError> {
    Ok(Json(s.publication.restore(p.user_id, project, &q).await?))
}
