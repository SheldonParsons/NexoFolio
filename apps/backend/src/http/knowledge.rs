//! Knowledge API: the catalogue tree, folder contents, and the curation
//! written about endpoints. Read only; every route needs login and access.

use crate::http::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{FolderId, ProjectId};
use nexofolio_contracts::knowledge::{KnowledgeError, KnowledgeReader};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct KnowledgeHttp {
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeReader>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeReader>,
) -> Router {
    Router::new()
        .route("/v1/projects/{project_id}/catalogue", get(catalogue))
        .route(
            "/v1/projects/{project_id}/folders/{folder_id}/endpoints",
            get(folder_endpoints),
        )
        .route("/v1/projects/{project_id}/unplaced", get(unplaced))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(KnowledgeHttp {
            projects,
            knowledge,
        })
        .layer(middleware::from_fn(no_store))
}

enum KnowledgeHttpError {
    Access(AccessError),
    NotFound,
    Unavailable,
}

impl From<AccessError> for KnowledgeHttpError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<KnowledgeError> for KnowledgeHttpError {
    fn from(error: KnowledgeError) -> Self {
        match error {
            KnowledgeError::NoSuchFolder => Self::NotFound,
            KnowledgeError::Unavailable => Self::Unavailable,
            _ => Self::Unavailable,
        }
    }
}

impl IntoResponse for KnowledgeHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "目录不存在"),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用",
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

async fn catalogue(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let catalogue = s.knowledge.catalogue(project).await?;
    Ok(Json(json!(catalogue)))
}

async fn folder_endpoints(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, folder)): Path<(ProjectId, FolderId)>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let endpoints = s.knowledge.folder_endpoints(project, Some(folder)).await?;
    Ok(Json(json!({ "endpoints": endpoints })))
}

async fn unplaced(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let endpoints = s.knowledge.folder_endpoints(project, None).await?;
    Ok(Json(json!({ "endpoints": endpoints })))
}
