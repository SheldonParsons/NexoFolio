use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::get,
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_contracts::{EnvironmentId, InterfaceId, ProjectId};
use nexofolio_knowledge::{AssessmentReader, DocumentQuery, DocumentReader};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone)]
pub struct DocumentsHttp {
    pub reader: Arc<dyn DocumentReader>,
    pub assessments: Arc<dyn AssessmentReader>,
    pub access: Arc<dyn PlatformAccess>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    environment_id: EnvironmentId,
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
    query: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvQuery {
    environment_id: EnvironmentId,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}
pub fn routes(state: DocumentsHttp) -> Router {
    Router::new()
        .route("/v1/projects/{project_id}/interfaces", get(list))
        .route(
            "/v1/projects/{project_id}/interfaces/{interface_id}",
            get(detail),
        )
        .route(
            "/v1/projects/{project_id}/interfaces/{interface_id}/observations",
            get(observations),
        )
        .route(
            "/v1/projects/{project_id}/observations/{ingestion_id}",
            get(observation),
        )
        .route(
            "/v1/projects/{project_id}/observations/{ingestion_id}/assessment",
            get(assessment),
        )
        .route(
            "/v1/projects/{project_id}/interfaces/{interface_id}/assessments",
            get(assessments),
        )
        .route_layer(middleware::from_fn_with_state(
            state.access.clone(),
            session_auth,
        ))
        .with_state(state)
        .layer(middleware::from_fn(super::access::no_store))
}
async fn list(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AccessError> {
    Ok(Json(
        s.reader
            .list(
                p.user_id,
                project,
                DocumentQuery {
                    environment_id: q.environment_id,
                    page: q.page,
                    limit: q.limit,
                    query: q.query,
                },
            )
            .await?,
    ))
}
async fn detail(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, InterfaceId)>,
    Query(q): Query<EnvQuery>,
) -> Result<Json<Value>, AccessError> {
    Ok(Json(
        s.reader
            .detail(p.user_id, project, q.environment_id, id)
            .await?,
    ))
}
async fn observations(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, InterfaceId)>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, AccessError> {
    Ok(Json(
        s.reader
            .observations(p.user_id, project, q.environment_id, id, q.page, q.limit)
            .await?,
    ))
}
async fn observation(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, Uuid)>,
) -> Result<Json<Value>, AccessError> {
    Ok(Json(s.reader.observation(p.user_id, project, id).await?))
}

async fn assessment(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, Uuid)>,
) -> Result<Json<nexofolio_contracts::ObservationAssessment>, AccessError> {
    Ok(Json(
        s.assessments.assessment(p.user_id, project, id).await?,
    ))
}
async fn assessments(
    State(s): State<DocumentsHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, InterfaceId)>,
    Query(q): Query<ListQuery>,
) -> Result<Json<nexofolio_contracts::ObservationAssessmentPage>, AccessError> {
    Ok(Json(
        s.assessments
            .assessments(
                p.user_id,
                project,
                id,
                DocumentQuery {
                    environment_id: q.environment_id,
                    page: q.page,
                    limit: q.limit,
                    query: q.query,
                },
            )
            .await?,
    ))
}
