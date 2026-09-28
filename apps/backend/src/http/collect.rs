//! `POST /v1/collect/batches`, the collect v1 contract (`contracts/collect/v1`).
//! No login: the batch names its project, and intake checks it.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use nexofolio_intake::{CollectError, Intake, MAX_BATCH_BYTES};
use std::sync::Arc;

/// `message` limit of `error.schema.json`.
const MAX_MESSAGE_CHARS: usize = 4096;

pub fn routes(intake: Arc<Intake>) -> Router {
    Router::new()
        .route("/v1/collect/batches", post(submit))
        .with_state(intake)
        .layer(DefaultBodyLimit::max(MAX_BATCH_BYTES))
}

async fn submit(
    State(intake): State<Arc<Intake>>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let result = match body {
        Ok(body) => intake.submit(&body).await,
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
