//! `POST /v1/collect/batches`, the collect v1 contract (`contracts/collect/v1`).
//! No login: the batch names its project, and intake checks it.

use super::collect_admission::{Admission, CLIENT_HEADER, DEFAULT_IN_FLIGHT};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, FromRequest, Request, State, rejection::BytesRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use nexofolio_intake::{CollectError, Intake, MAX_BATCH_BYTES};
use std::sync::Arc;
use uuid::Uuid;

/// `message` limit of `error.schema.json`.
const MAX_MESSAGE_CHARS: usize = 4096;

#[derive(Clone)]
struct CollectHttp {
    intake: Arc<Intake>,
    updates: Option<Arc<super::fetcher_updates::FetcherUpdates>>,
    telemetry: Arc<super::collect_telemetry::IntakeTelemetry>,
    admission: Arc<Admission>,
}

// Generic API-only harness constructor. Fetcher fails closed without the durable gate.
pub fn routes(intake: Arc<Intake>) -> Router {
    routes_with(intake, None)
}

pub fn routes_with_updates(
    intake: Arc<Intake>,
    ledger: nexofolio_intake_adapter::PostgresLedger,
) -> Router {
    routes_with(
        intake,
        Some(super::fetcher_updates::FetcherUpdates::new(ledger)),
    )
}
fn routes_with(
    intake: Arc<Intake>,
    updates: Option<Arc<super::fetcher_updates::FetcherUpdates>>,
) -> Router {
    routes_observed(
        intake,
        updates,
        Arc::new(super::collect_telemetry::IntakeTelemetry::default()),
        DEFAULT_IN_FLIGHT,
    )
}
pub(crate) fn routes_observed(
    intake: Arc<Intake>,
    updates: Option<Arc<super::fetcher_updates::FetcherUpdates>>,
    telemetry: Arc<super::collect_telemetry::IntakeTelemetry>,
    in_flight: usize,
) -> Router {
    let admission = Admission::new(in_flight, telemetry.clone());
    Router::new()
        .route("/v1/collect/batches", post(submit))
        .with_state(CollectHttp {
            intake,
            updates,
            telemetry,
            admission,
        })
        .layer(DefaultBodyLimit::max(MAX_BATCH_BYTES))
}

async fn submit(State(service): State<CollectHttp>, request: Request) -> Response {
    let started = std::time::Instant::now();
    let headers = request.headers().clone();
    let client = headers
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok());
    let Some(permit) = service.admission.enter(client) else {
        service.telemetry.observe(Some(0), 429, 0);
        return refused(CollectError::RateLimited {
            retry_after_secs: 1,
        });
    };
    let body = Bytes::from_request(request, &service).await;
    let bytes = body.as_ref().ok().map(Bytes::len);
    let runtime = tokio::runtime::Handle::current();
    let cancel = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let worker = service.clone();
    // The permit moves with the work: dropping an HTTP future cannot admit
    // replacement CPU work while an existing synchronous poll is still running.
    let response = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.block_on(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => refused(CollectError::Unavailable("cancelled")),
                response = submit_inner(&worker, headers, body, client) => response,
            }
        })
    })
    .await
    .unwrap_or_else(|_| refused(CollectError::Unavailable("collect_worker")));
    service.telemetry.observe(
        bytes,
        response.status().as_u16(),
        started.elapsed().as_millis() as u64,
    );
    response
}
async fn submit_inner(
    service: &CollectHttp,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
    client: Option<Uuid>,
) -> Response {
    let result = match body {
        Ok(body) => {
            if let Some(updates) = &service.updates {
                if let Err(response) = updates.check(&body, &headers).await {
                    return response;
                }
            } else if super::fetcher_updates::is_fetcher(&body, &headers) {
                return super::fetcher_updates::refusal(false);
            }
            service.intake.submit_for_client(&body, client).await
        }
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            Err(CollectError::BatchTooLarge)
        }
        Err(_) => Err(CollectError::InvalidBatch(
            "request body is unreadable".into(),
        )),
    };
    match result {
        Ok(receipt) => Json(receipt).into_response(),
        Err(error) => refused(error),
    }
}

fn refused(error: CollectError) -> Response {
    let status = match error {
        CollectError::InvalidBatch(_) => StatusCode::BAD_REQUEST,
        CollectError::BatchTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        CollectError::UnknownProject | CollectError::UnknownEnvironment => StatusCode::NOT_FOUND,
        CollectError::BatchIdReused => StatusCode::CONFLICT,
        CollectError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        CollectError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
    };
    let message: String = error.to_string().chars().take(MAX_MESSAGE_CHARS).collect();
    let mut response = (
        status,
        Json(serde_json::json!({"error": {"code": error.code(), "message": message}})),
    )
        .into_response();
    if let CollectError::RateLimited { retry_after_secs } = error {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs));
    }
    response
}
