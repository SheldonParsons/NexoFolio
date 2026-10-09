pub mod access;
pub mod collect;
pub mod curate;
mod curation_actor;
mod curation_operations;
pub(crate) mod curation_review;
pub mod downloads;
pub mod endpoints;
pub mod knowledge;
pub mod mcp_tokens;
pub(crate) mod profile;
pub mod service_addresses;
pub mod sites;
use crate::{
    mcp::{KnowledgeMcp, Navigation, require_token},
    wiring::Config,
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use nexofolio_access_contracts::McpTokenVerifier;
use nexofolio_common::DatabaseProbe;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{future::IntoFuture, io, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};

#[derive(Clone)]
pub struct RequestId(pub String);

#[derive(Clone)]
struct HealthState {
    database: Arc<dyn DatabaseProbe>,
    timeout: Duration,
}

async fn live() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"alive"}))
}

async fn ready(State(state): State<HealthState>) -> (StatusCode, Json<serde_json::Value>) {
    match tokio::time::timeout(state.timeout, state.database.check()).await {
        Ok(Ok(())) => (StatusCode::OK, Json(serde_json::json!({"status":"ready"}))),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status":"not_ready","component":"database"})),
        ),
    }
}

async fn identify(mut request: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("UUID is a valid header"),
    );
    response
}

fn request_deadline(path: &str, normal: Duration) -> Duration {
    if path.starts_with("/v1/projects/") && path.split('/').nth(4) == Some("curate") {
        Duration::from_secs(180)
    } else {
        normal
    }
}

async fn request_timeout(
    State(normal): State<Duration>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let duration = request_deadline(request.uri().path(), normal);
    tokio::time::timeout(duration, next.run(request)).await.map_err(|_| (
        StatusCode::REQUEST_TIMEOUT,
        Json(serde_json::json!({"error":{"code":"REQUEST_TIMEOUT","message":"处理超时，请稍后重试。"}})),
    ))
}

pub fn router(
    config: &Config,
    database: Arc<dyn DatabaseProbe>,
    verifier: Arc<dyn McpTokenVerifier>,
    shutdown: CancellationToken,
) -> Router {
    router_with_access(config, database, verifier, shutdown, None)
}

pub fn router_with_access(
    config: &Config,
    database: Arc<dyn DatabaseProbe>,
    verifier: Arc<dyn McpTokenVerifier>,
    shutdown: CancellationToken,
    access: Option<Router>,
) -> Router {
    router_with_navigation(config, database, verifier, shutdown, access, None)
}

pub fn router_with_navigation(
    config: &Config,
    database: Arc<dyn DatabaseProbe>,
    verifier: Arc<dyn McpTokenVerifier>,
    shutdown: CancellationToken,
    access: Option<Router>,
    navigation: Option<Navigation>,
) -> Router {
    let access_router = access.unwrap_or_default();
    let transport = StreamableHttpService::new(
        move || Ok(KnowledgeMcp::new(navigation.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_cancellation_token(shutdown)
            .with_allowed_hosts(config.mcp_allowed_hosts.clone())
            .with_allowed_origins(config.mcp_allowed_origins.clone())
            .with_max_request_body_bytes(1024 * 1024),
    );
    let mcp = Router::new()
        .nest_service("/mcp", transport)
        .route_layer(middleware::from_fn_with_state(verifier, require_token));
    Router::new().route("/health/live", get(live)).route("/health/ready", get(ready))
        .with_state(HealthState { database, timeout: config.database_timeout }).merge(mcp).merge(access_router)

        .merge(downloads::routes())
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn_with_state(config.request_timeout, request_timeout))
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request| {
            let id = request.extensions().get::<RequestId>().map(|i| i.0.as_str()).unwrap_or("unknown");
            // Never log query strings, bodies or Authorization headers.
            tracing::info_span!("http_request", request_id = id, method = %request.method(), path = request.uri().path())
        }))
        .layer(middleware::from_fn(identify))
}

pub async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: CancellationToken,
    grace: Duration,
) -> io::Result<()> {
    let serving = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned())
        .into_future();
    tokio::pin!(serving);
    tokio::select! {
        result = &mut serving => result,
        _ = shutdown.cancelled() => {
            tokio::time::timeout(grace, &mut serving).await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HTTP shutdown deadline exceeded"))?
        }
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn only_curation_gets_a_model_deadline() {
        let normal = Duration::from_secs(60);
        assert_eq!(
            request_deadline("/v1/projects/p/curate/organize", normal),
            Duration::from_secs(180)
        );
        assert_eq!(
            request_deadline("/v1/projects/p/curate/describe/e", normal),
            Duration::from_secs(180)
        );
        assert_eq!(request_deadline("/v1/projects/p/endpoints", normal), normal);
        assert_eq!(request_deadline("/health/ready", normal), normal);
    }
}
