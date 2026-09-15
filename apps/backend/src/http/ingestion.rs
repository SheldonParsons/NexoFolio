use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_application::IngestionService;
use nexofolio_contracts::Error;
use nexofolio_intake::{
    BatchReceipt, IngestionBatch, MAX_BATCH_BYTES, MAX_RECORD_BYTES, RecordReceipt, prepare_http,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub const ENVELOPE_SCHEMA: &str =
    include_str!("../../../../contracts/ingestion/envelope.schema.json");
pub const BATCH_SCHEMA: &str = include_str!("../../../../contracts/ingestion/batch.schema.json");
pub const RECEIPT_SCHEMA: &str =
    include_str!("../../../../contracts/ingestion/receipt.schema.json");
pub const CAPABILITIES_SCHEMA: &str =
    include_str!("../../../../contracts/ingestion/capabilities.schema.json");
#[derive(Clone)]
pub struct IngestionHttp {
    service: Arc<IngestionService>,
    access: Arc<dyn PlatformAccess>,
    envelope: Arc<jsonschema::Validator>,
    legacy_envelope: Arc<jsonschema::Validator>,
    record: Arc<jsonschema::Validator>,
    permits: Arc<Semaphore>,
    pub capture: Option<super::capture::CaptureHttp>,
}
fn compile(value: &Value) -> Result<jsonschema::Validator, Error> {
    jsonschema::options()
        .should_validate_formats(true)
        .build(value)
        .map_err(|_| Error::NotConfigured {
            capability: "ingestion_schema",
        })
}
impl IngestionHttp {
    pub fn new(
        service: Arc<IngestionService>,
        access: Arc<dyn PlatformAccess>,
    ) -> Result<Self, Error> {
        let envelope: Value = serde_json::from_str(ENVELOPE_SCHEMA).expect("bundled JSON Schema");
        let mut record: Value = serde_json::from_str(BATCH_SCHEMA).expect("bundled JSON Schema");
        let defs = record["$defs"].clone();
        record = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$defs":defs,"$ref":"#/$defs/Record"});
        Ok(Self {
            service,
            access,
            envelope: Arc::new(compile(&envelope)?),
            legacy_envelope: Arc::new(compile(
                &serde_json::from_str::<Value>(include_str!(
                    "../../../../contracts/ingestion/legacy/v1/envelope.schema.json"
                ))
                .expect("bundled v1 schema"),
            )?),
            record: Arc::new(compile(&record)?),
            permits: Arc::new(Semaphore::new(16)),
            capture: None,
        })
    }
}
pub fn capabilities_value() -> Value {
    json!({"schema_versions":["1","2"],"payload_versions":{"http_exchange":["1"]},"limits":{"max_records":50,"max_batch_bytes":MAX_BATCH_BYTES,"max_record_bytes":MAX_RECORD_BYTES,"max_concurrent_batches":16},"flush_defaults":{"max_records":20,"max_batch_bytes":4194304,"max_wait_ms":1000},"content_encodings":["identity"]})
}
pub fn routes(state: IngestionHttp) -> Router {
    Router::new()
        .route(
            "/v1/ingestion/capabilities",
            get(
                |State(s): State<IngestionHttp>,
                 axum::extract::Query(q): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| async move {
                    Json(
                        if q.get("schema_version").is_some_and(|v| v == "3") && s.capture.is_some()
                        {
                            super::capture::capabilities()
                        } else {
                            capabilities_value()
                        },
                    )
                },
            ),
        )
        .route("/v1/ingestion/batches", post(receive))
        .route_layer(middleware::from_fn_with_state(
            state.access.clone(),
            session_auth,
        ))
        .with_state(state)
        .layer(middleware::from_fn(|req: Request, next: Next| async move {
            let mut r = next.run(req).await;
            r.headers_mut()
                .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            r
        }))
}
fn error(status: StatusCode, code: &str, retryable: bool) -> Response {
    let mut r = (
        status,
        Json(json!({"error":{"code":code,"retryable":retryable}})),
    )
        .into_response();
    if retryable {
        r.headers_mut()
            .insert(header::RETRY_AFTER, "2".parse().unwrap());
    }
    r
}
async fn receive(
    State(state): State<IngestionHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    req: Request,
) -> Response {
    let Ok(permit) = state.permits.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "INGESTION_BUSY", true);
    };
    if req
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "UNSUPPORTED_CONTENT_ENCODING",
            false,
        );
    }
    if !req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON_REQUIRED", false);
    }
    let body = match to_bytes(req.into_body(), MAX_BATCH_BYTES).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "BATCH_TOO_LARGE", false),
    };
    if let Some(capture) = &state.capture {
        let response = capture.receive(principal, body.to_vec()).await;
        drop(permit);
        return response;
    }
    // CPU parsing is off the Tokio network executor; ownership of permit bounds cancelled work too.
    let envelope = state.envelope.clone();
    let legacy_envelope = state.legacy_envelope.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        let raw: Value = serde_json::from_slice(&body).map_err(|_| ())?;
        let validator = if raw["schema_version"] == "1" {
            &legacy_envelope
        } else {
            &envelope
        };
        if !validator.is_valid(&raw) {
            return Err(());
        }
        let b = serde_json::from_value::<IngestionBatch>(raw).map_err(|_| ())?;
        Ok((b, permit))
    })
    .await;
    let (batch, permit) = match parsed {
        Ok(Ok(v)) => v,
        _ => return error(StatusCode::BAD_REQUEST, "INVALID_ENVELOPE", false),
    };
    if let Err(e) = state.service.authorize(&principal, batch.project_id).await {
        return AccessError(e).into_response();
    }
    let record_schema = state.record.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let mut rejected = Vec::new();
        let mut records = Vec::new();
        for (i, r) in batch.records.iter().enumerate() {
            let id = r["record_id"].as_str().and_then(|s| s.parse().ok());
            let code = if serde_json::to_vec(r).map_or(true, |v| v.len() > MAX_RECORD_BYTES) {
                Some("RECORD_TOO_LARGE")
            } else if r["kind"].as_str().is_some_and(|s| s != "http_exchange") {
                Some("UNSUPPORTED_KIND")
            } else if r["payload_version"].as_str().is_some_and(|s| s != "1") {
                Some("UNSUPPORTED_PAYLOAD_VERSION")
            } else if !record_schema.is_valid(r) {
                Some("INVALID_RECORD")
            } else {
                None
            };
            if let Some(code) = code {
                rejected.push(RecordReceipt::reject(i, id, code));
                continue;
            }
            match prepare_http(&batch, i) {
                Ok(p) => records.push(p),
                Err(code) => rejected.push(RecordReceipt::reject(i, id, code)),
            }
        }
        (batch, records, rejected, permit)
    })
    .await;
    let (batch, records, mut results, _permit) = match prepared {
        Ok(v) => v,
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INGESTION_UNAVAILABLE",
                true,
            );
        }
    };
    let batch_id = batch.batch_id;
    let schema_version = batch.schema_version.clone();
    let mut environment = None;
    if !records.is_empty() {
        match state.service.admit(&principal, batch, records).await {
            Ok(mut result) => {
                environment = Some(result.environment);
                results.append(&mut result.receipts)
            }
            Err(Error::Unavailable {
                component: "ingestion",
            }) => {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "INGESTION_UNAVAILABLE",
                    true,
                );
            }
            Err(e) => return AccessError(e).into_response(),
        }
    }
    results.sort_by_key(|r| r.record_index);
    Json(BatchReceipt {
        schema_version,
        batch_id,
        results,
        environment,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_accepts_fixture_rejects_contract_drift() {
        let schema = compile(&serde_json::from_str::<Value>(BATCH_SCHEMA).unwrap()).unwrap();
        let mut v: Value = serde_json::from_str(include_str!(
            "../../../../contracts/ingestion/fixtures/http-batch.json"
        ))
        .unwrap();
        assert!(schema.is_valid(&v));
        v.as_object_mut().unwrap().remove("environment");
        assert!(!schema.is_valid(&v));
        v["environment"] = json!({"name":"开发环境"});
        v["records"][0]["record_id"] = json!("not-uuid");
        assert!(!schema.is_valid(&v));
    }
    #[test]
    fn capabilities_and_receipt_match_authoritative_schema() {
        assert!(
            compile(&serde_json::from_str::<Value>(CAPABILITIES_SCHEMA).unwrap())
                .unwrap()
                .is_valid(&capabilities_value())
        );
        let receipt = BatchReceipt {
            schema_version: "2".into(),
            batch_id: uuid::Uuid::new_v4(),
            results: vec![RecordReceipt::reject(0, None, "INVALID_RECORD")],
            environment: None,
        };
        assert!(
            compile(&serde_json::from_str::<Value>(RECEIPT_SCHEMA).unwrap())
                .unwrap()
                .is_valid(&serde_json::to_value(receipt).unwrap())
        );
    }
}
