use crate::{IngestionBatch, PreparedRecord};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

fn digest(v: &Value) -> Vec<u8> {
    Sha256::digest(serde_json::to_vec(v).expect("JSON values serialize")).to_vec()
}
fn media(headers: &Value) -> String {
    headers["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|p| {
            (p[0].as_str()?.eq_ignore_ascii_case("content-type")).then(|| {
                p[1].as_str()
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
        })
        .unwrap_or_default()
}
fn header_names(headers: &Value) -> std::collections::BTreeSet<String> {
    headers["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry[0].as_str().map(str::to_ascii_lowercase))
        .collect()
}
fn body_shape(body: &Value, media: &str) -> Option<Value> {
    if body["state"] == "none" {
        return Some(json!({"empty":true}));
    }
    if body["state"] != "complete" {
        return None;
    }
    let content = body["content"].as_str()?;
    let decoded = match body["encoding"].as_str()? {
        "text" => content.as_bytes().to_vec(),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(content)
            .ok()?,
        _ => return None,
    };
    if media == "application/json" || media.ends_with("+json") {
        let v: Value = serde_json::from_slice(&decoded).ok()?;
        {
            let observed = nexofolio_contracts::observe_json(&v);
            (!observed
                .limitations
                .iter()
                .any(|n| n == "STRUCTURE_EXTRACTION_LIMIT"))
            .then_some(observed.schema)
        }
    } else {
        None
    }
}
/// Must be called after machine-schema validation. Returns a non-sensitive reason code on errors.
pub fn prepare_http(
    batch: &IngestionBatch,
    index: usize,
) -> std::result::Result<PreparedRecord, &'static str> {
    let raw = &batch.records[index];
    let payload = &raw["payload"];
    let request = &payload["request"];
    let response = &payload["response"];
    let id = raw["record_id"]
        .as_str()
        .and_then(|s| s.parse::<Uuid>().ok())
        .ok_or("INVALID_RECORD")?;
    let url = Url::parse(request["url"].as_str().ok_or("INVALID_RECORD")?)
        .map_err(|_| "INVALID_RECORD")?;
    if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
        return Err("INVALID_RECORD");
    }
    for body in [&request["body"], &response["body"]] {
        let state = body["state"].as_str().ok_or("INVALID_RECORD")?;
        let encoding = body["encoding"].as_str().ok_or("INVALID_RECORD")?;
        let content = body["content"].as_str().ok_or("INVALID_RECORD")?;
        if (state == "none" && (encoding != "none" || !content.is_empty() || body["bytes"] != 0))
            || (encoding == "none" && !content.is_empty())
            || (state == "complete" && encoding == "none")
        {
            return Err("INVALID_BODY_STATE");
        }
        if encoding == "base64"
            && base64::engine::general_purpose::STANDARD
                .decode(content)
                .is_err()
        {
            return Err("INVALID_BODY_ENCODING");
        }
    }
    let method = request["method"].as_str().ok_or("INVALID_RECORD")?;
    let identity_key = format!("{method} {}", url.path());
    let structural_projection = http_projection(raw);
    let structural_hash = structural_projection.as_ref().map(digest);
    // Scope and captured payload participate in immutable observation retries; batch ID doesn't.
    let content_hash = if batch.schema_version == "1" {
        digest(
            &json!({"schema_version":batch.schema_version,"project_id":batch.project_id,"service_key":batch.service_key,"source":batch.source,"record":raw}),
        )
    } else {
        digest(
            &json!({"schema_version":batch.schema_version,"project_id":batch.project_id,"source":batch.source,"record":raw}),
        )
    };
    Ok(PreparedRecord {
        index,
        record_id: id,
        content_hash,
        identity_key,
        path_identity: None,
        structural_hash,
        structural_projection,
        raw: raw.clone(),
    })
}

/// Reconstructs only the current head's comparison projection for algorithm upgrades.
pub fn http_projection(raw: &Value) -> Option<Value> {
    let payload = raw.get("payload")?;
    let request = payload.get("request")?;
    let response = payload.get("response")?;
    let url = Url::parse(request["url"].as_str()?).ok()?;
    let req_media = media(&request["headers"]);
    let res_media = media(&response["headers"]);
    let header_known = |h: &Value| matches!(h["state"].as_str(), Some("page-visible" | "complete"));
    let confident = request["url_truncated"] == false
        && response["state"] == "complete"
        && response["status"].is_number()
        && header_known(&request["headers"])
        && header_known(&response["headers"]);
    let query: std::collections::BTreeSet<_> =
        url.query_pairs().map(|(k, _)| k.to_string()).collect();
    if confident {
        body_shape(&request["body"],&req_media).zip(body_shape(&response["body"],&res_media)).map(|(a,b)|json!({"algorithm":nexofolio_contracts::STRUCTURE_ALGORITHM,"query_keys":query,"request_headers":header_names(&request["headers"]),"response_headers":header_names(&response["headers"]),"request_media":req_media,"request_body":a,"status":response["status"],"response_media":res_media,"response_body":b}))
    } else {
        None
    }
}
pub fn http_projection_covers(known: &Value, incoming: &Value) -> bool {
    let (Some(a), Some(b)) = (known.as_object(), incoming.as_object()) else {
        return false;
    };
    a.keys().eq(b.keys())
        && a.iter().all(|(key, value)| {
            if key == "request_body" || key == "response_body" {
                nexofolio_contracts::observed_schema_covers(value, &b[key])
            } else {
                Some(value) == b.get(key)
            }
        })
}

/// Apply the project rule once per prepared record, after authorization and before head locking.
pub fn apply_path_policy(record: &mut PreparedRecord, policy: &nexofolio_contracts::PathPolicy) {
    let request = &record.raw["payload"]["request"];
    let url = Url::parse(request["url"].as_str().expect("validated URL")).expect("validated URL");
    let policy = if request["url_truncated"] == true {
        nexofolio_contracts::PathPolicy {
            enabled: false,
            ..policy.clone()
        }
    } else {
        policy.clone()
    };
    let path = nexofolio_contracts::identify_path(url.path(), &policy);
    record.identity_key = format!(
        "{} {}",
        request["method"].as_str().expect("validated method"),
        path.template
    );
    record.path_identity = Some(path);
}

/// Shared legacy hashing ensures old immutable retry receipts survive evidence rollout.
pub fn prepare_capture(
    batch: &nexofolio_contracts::CaptureBatch,
    index: usize,
) -> std::result::Result<crate::PreparedCapture, &'static str> {
    let raw = batch.records[index].clone();
    let record: nexofolio_contracts::CaptureRecord =
        serde_json::from_value(raw.clone()).map_err(|_| "INVALID_RECORD")?;
    let legacy = IngestionBatch {
        schema_version: batch.schema_version.clone(),
        batch_id: batch.batch_id,
        project_id: batch.project_id,
        environment: batch.environment.clone(),
        service_key: batch.service_key.clone(),
        source: crate::Producer {
            r#type: batch.source.r#type.clone(),
            instance_id: batch.source.instance_id,
        },
        records: vec![raw.clone()],
    };
    let http = if matches!(record.kind, nexofolio_contracts::CaptureKind::HttpExchange) {
        Some({
            let mut http = prepare_http(&legacy, 0)?;
            http.index = index;
            http
        })
    } else {
        None
    };
    let hash=http.as_ref().map(|h|h.content_hash.clone()).unwrap_or_else(||digest(&json!({"schema_version":batch.schema_version,"project_id":batch.project_id,"source":batch.source,"record":raw})));
    Ok(crate::PreparedCapture {
        index,
        record,
        raw,
        content_hash: hash,
        http,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch() -> IngestionBatch {
        serde_json::from_str(include_str!(
            "../../../contracts/ingestion/fixtures/http-batch.json"
        ))
        .unwrap()
    }
    #[test]
    fn path_rules_do_not_change_raw_payload_or_immutable_retry_hash() {
        let mut batch = batch();
        batch.records[0]["payload"]["request"]["url"] = json!("https://api.test/v1/users/123");
        let mut row = prepare_http(&batch, 0).unwrap();
        let hash = row.content_hash.clone();
        let raw = row.raw.clone();
        apply_path_policy(&mut row, &nexofolio_contracts::PathPolicy::default());
        assert_eq!(row.identity_key, "GET /v1/users/{param1}");
        assert_eq!(row.content_hash, hash);
        assert_eq!(row.raw, raw);
        apply_path_policy(
            &mut row,
            &nexofolio_contracts::PathPolicy {
                enabled: false,
                ..Default::default()
            },
        );
        assert_eq!(row.identity_key, "GET /v1/users/123");
        batch.records[0]["payload"]["request"]["url_truncated"] = json!(true);
        let mut row = prepare_http(&batch, 0).unwrap();
        apply_path_policy(&mut row, &Default::default());
        assert_eq!(row.identity_key, "GET /v1/users/123");
    }
    #[test]
    fn directional_comparison_checks_other_fields_and_keeps_unknown_conservative() {
        let raw = batch().records[0].clone();
        let full = http_projection(&raw).unwrap();
        let mut next = raw.clone();
        next["payload"]["response"]["body"]["content"] = json!(r#"{"items":[]}"#);
        let empty = http_projection(&next).unwrap();
        assert!(http_projection_covers(&full, &empty));
        assert!(!http_projection_covers(&empty, &full));
        next["payload"]["response"]["body"]["content"] = json!(r#"{"items":[],"new":true}"#);
        assert!(!http_projection_covers(
            &full,
            &http_projection(&next).unwrap()
        ));
        next["payload"]["response"]["body"]["content"] = json!(r#"{"items":null}"#);
        assert!(http_projection(&next).is_some());
        next["payload"]["response"]["body"]["state"] = json!("truncated");
        assert!(http_projection(&next).is_none());
    }
    #[test]
    fn legacy_retry_digest_matches_original_v1_wire() {
        let batch: IngestionBatch = serde_json::from_str(include_str!(
            "../../../contracts/ingestion/legacy/v1/fixtures/http-batch.json"
        ))
        .unwrap();
        let prepared = prepare_http(&batch, 0).unwrap();
        let digest = prepared
            .content_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "6b398d12c53e6cc3559421fb5093ea5cc0aed578d8354a7478295efe4766c29b"
        );
    }
    #[test]
    fn structural_comparison_ignores_values_not_fields_or_status() {
        let a = batch();
        let fa = prepare_http(&a, 0).unwrap();
        let mut b = a.clone();
        b.records[0]["payload"]["response"]["body"]["content"] =
            json!("{\"items\":[{\"name\":\"different\",\"id\":42},{\"id\":9,\"name\":\"more\"}]}");
        let fb = prepare_http(&b, 0).unwrap();
        assert_eq!(fa.structural_hash, fb.structural_hash);
        assert_ne!(fa.content_hash, fb.content_hash);
        let mut changed_type = a.clone();
        changed_type.records[0]["payload"]["response"]["body"]["content"] =
            json!("{\"items\":[{\"id\":\"1\",\"name\":\"example\"}]}");
        assert_ne!(
            fa.structural_hash,
            prepare_http(&changed_type, 0).unwrap().structural_hash
        );
        b.records[0]["payload"]["response"]["status"] = json!(201);
        assert_ne!(
            fa.structural_hash,
            prepare_http(&b, 0).unwrap().structural_hash
        );
        b.records[0]["payload"]["response"]["body"]["content"] =
            json!("{\"items\":[{\"id\":1}, {\"added\":true}]}");
        assert_ne!(
            fa.structural_hash,
            prepare_http(&b, 0).unwrap().structural_hash
        );
    }
    #[test]
    fn inconclusive_content_never_gets_a_structural_hit() {
        let mut b = batch();
        b.records[0]["payload"]["response"]["body"]["content"] = json!("invalid json");
        assert!(prepare_http(&b, 0).unwrap().structural_hash.is_none());
        let mut b = batch();
        b.records[0]["payload"]["response"]["state"] = json!("truncated");
        assert!(prepare_http(&b, 0).unwrap().structural_hash.is_none());
    }
}
