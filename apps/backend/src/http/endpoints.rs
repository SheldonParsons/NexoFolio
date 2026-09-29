//! The project's endpoint document: the list, one endpoint's facts, and the
//! raw call behind an example. Read only; every route needs login and access
//! to the project.
//!
//! Examples are not redacted, so they never reach logs or error bodies.

use crate::http::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::endpoint::{EndpointError, EndpointFacts, EndpointReader};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct EndpointsHttp {
    projects: Arc<dyn ProjectAccess>,
    endpoints: Arc<dyn EndpointReader>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    endpoints: Arc<dyn EndpointReader>,
) -> Router {
    Router::new()
        .route("/v1/projects/{project_id}/endpoints", get(list))
        .route(
            "/v1/projects/{project_id}/endpoints/{endpoint_id}",
            get(facts),
        )
        .route(
            "/v1/projects/{project_id}/endpoints/{endpoint_id}/examples/{example_id}",
            get(example),
        )
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(EndpointsHttp {
            projects,
            endpoints,
        })
        .layer(middleware::from_fn(no_store))
}

/// Free text over the path template and the method, case-insensitive.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    q: String,
}

enum EndpointsError {
    Access(AccessError),
    /// The endpoint belongs to another project.
    Forbidden,
    NotFound,
    Unavailable,
}

impl From<AccessError> for EndpointsError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<EndpointError> for EndpointsError {
    fn from(_: EndpointError) -> Self {
        Self::Unavailable
    }
}

impl IntoResponse for EndpointsError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "PROJECT_ACCESS_DENIED",
                "该接口属于其他项目",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "接口不存在"),
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

async fn list(
    State(s): State<EndpointsHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(search): Query<Search>,
) -> Result<Json<Value>, EndpointsError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let mut endpoints = s.endpoints.list(project).await?;
    let needle = search.q.trim().to_lowercase();
    if !needle.is_empty() {
        endpoints.retain(|endpoint| {
            endpoint.path_template.to_lowercase().contains(&needle)
                || endpoint.method.to_lowercase().contains(&needle)
        });
    }
    Ok(Json(json!({ "endpoints": endpoints })))
}

/// The endpoint's facts. An alias answers with the endpoint it was merged
/// into, so old links keep working.
async fn facts(
    State(s): State<EndpointsHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, endpoint)): Path<(ProjectId, EndpointId)>,
) -> Result<Json<Value>, EndpointsError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    Ok(Json(json!(facts_of(&s, project, endpoint).await?)))
}

/// Resolves an endpoint of `project`, aliases included.
async fn facts_of(
    s: &EndpointsHttp,
    project: ProjectId,
    endpoint: EndpointId,
) -> Result<EndpointFacts, EndpointsError> {
    let facts = s
        .endpoints
        .get(endpoint)
        .await?
        .ok_or(EndpointsError::NotFound)?;
    if facts.summary.project_id != project {
        return Err(EndpointsError::Forbidden);
    }
    Ok(facts)
}

/// The whole call an example stands for, exactly as it was captured.
async fn example(
    State(s): State<EndpointsHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, endpoint, example)): Path<(ProjectId, EndpointId, String)>,
) -> Result<Json<Value>, EndpointsError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let facts = facts_of(&s, project, endpoint).await?;
    let observation = s
        .endpoints
        .example(facts.summary.id, &example)
        .await?
        .ok_or(EndpointsError::NotFound)?;
    Ok(Json(json!({ "observation": observation })))
}
