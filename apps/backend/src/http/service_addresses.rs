//! Which service addresses count as the project's own (0003 §4.2). Both
//! routes need login and access to the project.

use crate::http::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::ProjectId;
use nexofolio_contracts::endpoint::{EndpointError, ServiceAddress, ServiceAddresses, Verdict};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct AddressesHttp {
    projects: Arc<dyn ProjectAccess>,
    addresses: Arc<dyn ServiceAddresses>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    addresses: Arc<dyn ServiceAddresses>,
) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project_id}/service-addresses",
            get(list).put(decide),
        )
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(AddressesHttp {
            projects,
            addresses,
        })
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideBody {
    address: ServiceAddress,
    /// `null` clears the manual verdict: the address is the project's own again.
    verdict: Option<Verdict>,
}

enum AddressesError {
    Access(AccessError),
    Unavailable,
}

impl From<AccessError> for AddressesError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<EndpointError> for AddressesError {
    fn from(_: EndpointError) -> Self {
        Self::Unavailable
    }
}

impl IntoResponse for AddressesError {
    fn into_response(self) -> Response {
        match self {
            Self::Access(error) => error.into_response(),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(
                    json!({"error": {"code": "SERVICE_UNAVAILABLE", "message": "服务暂时不可用"}}),
                ),
            )
                .into_response(),
        }
    }
}

async fn list(
    State(s): State<AddressesHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, AddressesError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let addresses = s.addresses.list(project).await?;
    Ok(Json(json!({ "addresses": addresses })))
}

async fn decide(
    State(s): State<AddressesHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Json(body): Json<DecideBody>,
) -> Result<StatusCode, AddressesError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    s.addresses
        .decide(project, body.address, body.verdict)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
