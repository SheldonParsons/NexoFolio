use super::access::{AccessError, session_auth};
use axum::{
    Extension, Json, Router,
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_application::CaptureGateway;
use nexofolio_contracts::*;
use nexofolio_intake::{MAX_BATCH_BYTES, MAX_RECORD_BYTES, prepare_capture};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone)]
pub struct CaptureHttp {
    pub store: Arc<dyn CaptureGateway>,
    pub access: Arc<dyn PlatformAccess>,
    envelopes: Arc<Vec<jsonschema::Validator>>,
    records: Arc<Vec<jsonschema::Validator>>,
    asset_permits: Arc<tokio::sync::Semaphore>,
}
fn compile(value: Value) -> Result<jsonschema::Validator> {
    jsonschema::options()
        .should_validate_formats(true)
        .build(&value)
        .map_err(|_| Error::NotConfigured {
            capability: "capture_schema",
        })
}
impl CaptureHttp {
    pub fn new(store: Arc<dyn CaptureGateway>, access: Arc<dyn PlatformAccess>) -> Result<Self> {
        let v3: Value = serde_json::from_str(include_str!(
            "../../../../contracts/capture/batch.schema.json"
        ))
        .unwrap();
        let v2: Value = serde_json::from_str(include_str!(
            "../../../../contracts/ingestion/envelope.schema.json"
        ))
        .unwrap();
        let v1: Value = serde_json::from_str(include_str!(
            "../../../../contracts/ingestion/legacy/v1/envelope.schema.json"
        ))
        .unwrap();
        let old: Value = serde_json::from_str(include_str!(
            "../../../../contracts/ingestion/batch.schema.json"
        ))
        .unwrap();
        let old_record = json!({"$defs":old["$defs"],"$ref":"#/$defs/Record"});
        Ok(Self {
            store,
            access,
            asset_permits: Arc::new(tokio::sync::Semaphore::new(4)),
            envelopes: Arc::new(vec![compile(v1)?, compile(v2)?, compile(v3)?]),
            records: Arc::new(vec![
                compile(old_record)?,
                compile(
                    serde_json::from_str(include_str!(
                        "../../../../contracts/capture/record.schema.json"
                    ))
                    .unwrap(),
                )?,
            ]),
        })
    }
    pub async fn receive(&self, principal: SessionPrincipal, bytes: Vec<u8>) -> Response {
        let envelopes = self.envelopes.clone();
        let records = self.records.clone();
        let parsed = tokio::task::spawn_blocking(move || -> Result<_> {
            let raw: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidInput {
                message: "invalid capture JSON".into(),
            })?;
            let version = match raw["schema_version"].as_str() {
                Some("1") => 0,
                Some("2") => 1,
                Some("3") => 2,
                _ => {
                    return Err(Error::InvalidInput {
                        message: "unsupported capture schema".into(),
                    });
                }
            };
            if !envelopes[version].is_valid(&raw) {
                return Err(Error::InvalidInput {
                    message: "invalid capture envelope".into(),
                });
            }
            let batch: CaptureBatch =
                serde_json::from_value(raw).map_err(|_| Error::InvalidInput {
                    message: "invalid capture envelope".into(),
                })?;
            Ok((batch, version))
        })
        .await;
        let (batch, version) = match parsed {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return AccessError(e).into_response(),
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        if let Err(e) = self
            .access
            .require_project(&principal, batch.project_id)
            .await
        {
            return AccessError(e).into_response();
        }
        let prepared = tokio::task::spawn_blocking(move || {
            let mut prepared = Vec::new();
            let mut rejected = Vec::new();
            for (index, record) in batch.records.iter().enumerate() {
                let id = record["record_id"].as_str().and_then(|s| s.parse().ok());
                let error =
                    if serde_json::to_vec(record).map_or(true, |v| v.len() > MAX_RECORD_BYTES) {
                        Some("RECORD_TOO_LARGE")
                    } else if !records[if version == 2 { 1 } else { 0 }].is_valid(record) {
                        Some("INVALID_RECORD")
                    } else {
                        None
                    };
                let next = if let Some(code) = error {
                    Err(code)
                } else {
                    prepare_capture(&batch, index)
                };
                match next {
                    Ok(r) => prepared.push(r),
                    Err(code) => rejected.push(CaptureReceipt {
                        record_index: index,
                        record_id: id,
                        status: CaptureReceiptStatus::Rejected,
                        reason_code: code.into(),
                        retryable: false,
                        observation_id: None,
                        ingestion_id: None,
                        structure: "not_applicable".into(),
                        replayed: false,
                    }),
                };
            }
            (batch, prepared, rejected)
        })
        .await;
        let (batch, prepared, mut rejected) = match prepared {
            Ok(v) => v,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        let version = batch.schema_version.clone();
        let batch_id = batch.batch_id;
        let result = if prepared.is_empty() {
            CaptureBatchReceipt {
                schema_version: version.clone(),
                batch_id,
                environment: None,
                results: vec![],
            }
        } else {
            match self
                .store
                .admit_captures(principal.user_id, &principal.instance, batch, prepared)
                .await
            {
                Ok(v) => v,
                Err(e) => return AccessError(e).into_response(),
            }
        };
        let mut result = result;
        result.results.append(&mut rejected);
        result.results.sort_by_key(|r| r.record_index);
        if version == "3" {
            Json(result).into_response()
        } else {
            let rows:Vec<_>=result.results.into_iter().map(|r|{
                let (status,code)=if r.reason_code=="OBSERVATION_STORED" {if r.structure=="duplicate"{("ignored".to_owned(),"DUPLICATE_CURRENT_STRUCTURE".to_owned())}else{("accepted".to_owned(),"FORWARDED".to_owned())}}else{(serde_json::to_value(&r.status).unwrap().as_str().unwrap().to_owned(),r.reason_code)};
                json!({"record_index":r.record_index,"record_id":r.record_id,"status":status,"reason_code":code,"retryable":r.retryable,"ingestion_id":r.ingestion_id,"replayed":r.replayed})
            }).collect();
            Json(json!({"schema_version":version,"batch_id":batch_id,"environment":result.environment,"results":rows})).into_response()
        }
    }
}
pub fn capabilities() -> Value {
    json!({"schema_versions":["1","2","3"],"payload_versions":{"http_exchange":["1"],"page_context":["1"],"interaction":["1"],"ui_snapshot":["1"],"image_reference":["1"]},"limits":{"max_records":50,"max_batch_bytes":MAX_BATCH_BYTES,"max_record_bytes":MAX_RECORD_BYTES,"max_concurrent_batches":16},"flush_defaults":{"max_records":20,"max_batch_bytes":4194304,"max_wait_ms":1000},"content_encodings":["identity"],"assets":{"max_bytes":8388608,"media_types":["image/png","image/jpeg","image/webp"]}})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceQuery {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
    environment_id: Option<EnvironmentId>,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}
pub fn routes(s: CaptureHttp) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project}/assets/{asset}",
            get(asset).put(upload),
        )
        .route(
            "/v1/projects/{project}/capture-observations/{observation}",
            get(observation),
        )
        .route("/v1/projects/{project}/evidence", get(evidence))
        .route("/v1/projects/{project}/evidence/{fact}", get(fact))
        .route_layer(middleware::from_fn_with_state(
            s.access.clone(),
            session_auth,
        ))
        .with_state(s)
        .layer(middleware::from_fn(
            |r: Request, n: axum::middleware::Next| async move {
                let mut r = n.run(r).await;
                r.headers_mut()
                    .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
                r.headers_mut()
                    .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
                r
            },
        ))
}
async fn upload(
    State(s): State<CaptureHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, Uuid)>,
    req: Request,
) -> std::result::Result<Json<AssetReceipt>, AccessError> {
    let _permit = s
        .asset_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Unavailable {
            component: "asset_backpressure",
        })?;
    s.access.require_project(&p, project).await?;
    let media = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    let body = to_bytes(req.into_body(), 8 * 1024 * 1024)
        .await
        .map_err(|_| Error::InvalidInput {
            message: "asset exceeds8MiB".into(),
        })?;
    Ok(Json(
        s.store
            .put_asset(p.user_id, project, id, &media, body.to_vec())
            .await?,
    ))
}
async fn asset(
    State(s): State<CaptureHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, Uuid)>,
) -> std::result::Result<Response, AccessError> {
    let _permit = s
        .asset_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Unavailable {
            component: "asset_backpressure",
        })?;
    let (media, bytes) = s.store.asset(p.user_id, project, id).await?;
    Ok(([(header::CONTENT_TYPE, media)], bytes).into_response())
}
async fn observation(
    State(s): State<CaptureHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, Uuid)>,
) -> std::result::Result<Json<CaptureObservation>, AccessError> {
    Ok(Json(s.store.observation(p.user_id, project, id).await?))
}
async fn evidence(
    State(s): State<CaptureHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(q): Query<EvidenceQuery>,
) -> std::result::Result<Json<EvidencePage>, AccessError> {
    Ok(Json(
        s.store
            .facts(p.user_id, project, q.page, q.limit, q.environment_id)
            .await?,
    ))
}

async fn fact(
    State(s): State<CaptureHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path((project, fact)): Path<(ProjectId, Uuid)>,
) -> std::result::Result<Json<EvidenceFact>, AccessError> {
    Ok(Json(s.store.fact(p.user_id, project, fact).await?))
}
