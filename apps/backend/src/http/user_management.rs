//! Read-only registered user directory. User mutations are deliberately absent.
use super::{
    access::{AccessError, no_store},
    management::administrator,
};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{SessionPrincipal, Sessions, UserDirectory};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserQuery {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "limit")]
    limit: u32,
    #[serde(default)]
    q: String,
}
fn first() -> u32 {
    1
}
fn limit() -> u32 {
    20
}

async fn list(
    State(directory): State<Arc<dyn UserDirectory>>,
    Extension(principal): Extension<SessionPrincipal>,
    Query(query): Query<UserQuery>,
) -> Result<Response, AccessError> {
    Ok(Json(
        directory
            .list_registered_users(&principal, query.page, query.limit, &query.q)
            .await?,
    )
    .into_response())
}

pub(crate) fn routes(sessions: Arc<dyn Sessions>, directory: Arc<dyn UserDirectory>) -> Router {
    Router::new()
        .route("/v1/user-management/users", get(list))
        .route_layer(middleware::from_fn_with_state(sessions, administrator))
        .with_state(directory)
        .layer(middleware::from_fn(no_store))
}
