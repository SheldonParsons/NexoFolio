//! Account-wide MCP credentials, managed only through a web session.
use super::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use nexofolio_access_contracts::{
    CreateMcpToken, McpTokenPage, McpTokens, SessionPrincipal, Sessions,
};
use nexofolio_common::{Error, TokenId};
use rmcp::ServerHandler;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn routes(sessions: Arc<dyn Sessions>, tokens: Arc<dyn McpTokens>) -> Router {
    Router::new()
        .route("/v1/auth/mcp-connection", get(connection))
        .route("/v1/auth/mcp-tokens", get(list).post(create))
        .route("/v1/auth/mcp-tokens/{id}", delete(revoke))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(tokens)
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(no_store))
}
struct TokenError(Error);
impl From<Error> for TokenError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for TokenError {
    fn into_response(self) -> Response {
        match self.0 {
            Error::InvalidInput { message } => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":{"code":"VALIDATION_ERROR","message":message}})),
            )
                .into_response(),
            e => AccessError(e).into_response(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pagination {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}
async fn connection(
    State(tokens): State<Arc<dyn McpTokens>>,
    Extension(owner): Extension<SessionPrincipal>,
) -> Result<Json<serde_json::Value>, TokenError> {
    // Read the versions declared by the actual MCP handler, not a separate UI list.
    let versions = crate::mcp::KnowledgeMcp::default()
        .supported_protocol_versions()
        .iter()
        .rev()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "transport": "Streamable HTTP",
        "protocol_versions": versions,
        "accessible_projects": tokens.mcp_accessible_projects(&owner).await?,
    })))
}
async fn list(
    State(tokens): State<Arc<dyn McpTokens>>,
    Extension(owner): Extension<SessionPrincipal>,
    Query(q): Query<Pagination>,
) -> Result<Json<McpTokenPage>, TokenError> {
    Ok(Json(tokens.list_mcp_tokens(&owner, q.page, q.limit).await?))
}
async fn create(
    State(tokens): State<Arc<dyn McpTokens>>,
    Extension(owner): Extension<SessionPrincipal>,
    Json(input): Json<CreateMcpToken>,
) -> Result<Response, TokenError> {
    let result = tokens.create_mcp_token(&owner, input).await?;
    Ok((StatusCode::CREATED, Json(json!({"credential":result.credential,"token":result.token.expose(),"token_type":"Bearer"}))).into_response())
}
async fn revoke(
    State(tokens): State<Arc<dyn McpTokens>>,
    Extension(owner): Extension<SessionPrincipal>,
    Path(id): Path<TokenId>,
) -> Result<StatusCode, TokenError> {
    tokens.revoke_owned_mcp_token(&owner, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
