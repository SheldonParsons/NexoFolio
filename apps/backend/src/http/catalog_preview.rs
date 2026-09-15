use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::get,
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_contracts::{CatalogPreviewDetail, CatalogPreviewPage, JobId, ProjectId};
use nexofolio_knowledge::CatalogPreviewReader;
use serde::Deserialize;
use std::sync::Arc;
#[derive(Clone)]
pub struct CatalogPreviewHttp {
    pub reader: Arc<dyn CatalogPreviewReader>,
    pub access: Arc<dyn PlatformAccess>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
    status: Option<nexofolio_contracts::PreviewStatus>,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}
pub fn routes(state: CatalogPreviewHttp) -> Router {
    Router::new()
        .route("/v1/projects/{project_id}/catalog-previews", get(list))
        .route(
            "/v1/projects/{project_id}/catalog-previews/{task_id}",
            get(detail),
        )
        .route_layer(middleware::from_fn_with_state(
            state.access.clone(),
            session_auth,
        ))
        .with_state(state)
        .layer(middleware::from_fn(super::access::no_store))
}
async fn list(
    State(s): State<CatalogPreviewHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<ListQuery>,
) -> Result<Json<CatalogPreviewPage>, AccessError> {
    Ok(Json(
        s.reader
            .list_previews(p.user_id, project, q.page, q.limit, q.status)
            .await?,
    ))
}
async fn detail(
    State(s): State<CatalogPreviewHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, task)): Path<(ProjectId, JobId)>,
) -> Result<Json<CatalogPreviewDetail>, AccessError> {
    Ok(Json(
        s.reader.preview_detail(p.user_id, project, task).await?,
    ))
}
