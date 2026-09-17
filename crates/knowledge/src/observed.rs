//! Deterministic observed schema extraction. Never infers required fields or descriptions.
use async_trait::async_trait;
use nexofolio_contracts::{EnvironmentId, InterfaceId, ProjectId, Result, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedDefinition {
    pub extractor_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_view: Option<String>,
    pub method: String,
    pub path: String,
    pub request: Value,
    pub response: Value,
    pub limitations: Vec<String>,
}
#[derive(Clone)]
pub struct ClaimedObservation {
    pub ingestion_id: Uuid,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
    pub identity_key: String,
    pub path_identity: Option<nexofolio_contracts::PathIdentity>,
    pub generation: i64,
    pub raw: Value,
}
#[derive(Debug, Serialize)]
pub struct ProcessingResult {
    pub ingestion_id: Uuid,
    pub interface_id: InterfaceId,
    pub outcome: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentQuery {
    pub environment_id: EnvironmentId,
    pub page: u32,
    pub limit: u32,
    pub query: Option<String>,
}

#[async_trait]
pub trait ObservationProcessor: Send + Sync {
    async fn claim(&self) -> Result<Option<ClaimedObservation>>;
    async fn finish(
        &self,
        claim: &ClaimedObservation,
        definition: ObservedDefinition,
    ) -> Result<ProcessingResult>;
    async fn fail(&self, claim: &ClaimedObservation, code: &str) -> Result<()>;
    async fn retry_failed(&self, ingestion_id: Uuid) -> Result<()>;
}
#[async_trait]
pub trait DocumentReader: Send + Sync {
    async fn list(&self, user: UserId, project: ProjectId, query: DocumentQuery) -> Result<Value>;
    async fn detail(
        &self,
        user: UserId,
        project: ProjectId,
        environment: EnvironmentId,
        interface: InterfaceId,
    ) -> Result<Value>;
    async fn observations(
        &self,
        user: UserId,
        project: ProjectId,
        environment: EnvironmentId,
        interface: InterfaceId,
        page: u32,
        limit: u32,
    ) -> Result<Value>;
    async fn observation(
        &self,
        user: UserId,
        project: ProjectId,
        ingestion_id: Uuid,
    ) -> Result<Value>;
}

pub fn extract_observed(raw: &Value) -> Result<ObservedDefinition> {
    let invalid = || nexofolio_contracts::Error::invalid("INVALID_OBSERVATION");
    let p = raw.get("payload").ok_or_else(invalid)?;
    let req = p.get("request").ok_or_else(invalid)?;
    let res = p.get("response").ok_or_else(invalid)?;
    let url = Url::parse(req["url"].as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let method = req["method"].as_str().ok_or_else(invalid)?.to_owned();
    let mut limitations = vec!["OBSERVED_ONLY_NOT_A_CONFIRMED_CONTRACT".into()];
    let query: std::collections::BTreeSet<_> = url
        .query_pairs()
        .map(|(name, _)| name.to_string())
        .collect();
    let params: Vec<_> = query
        .into_iter()
        .map(|name| json!({"name":name,"in":"query","observed_schema":{"type":"string"}}))
        .collect();
    let request = json!({"parameters":params,"headers":headers(&req["headers"]),"body":body(&req["body"],&req["headers"],"request",&mut limitations)});
    let response = json!({"status":res["status"],"capture_state":res["state"],"headers":headers(&res["headers"]),"body":body(&res["body"],&res["headers"],"response",&mut limitations)});
    if req["url_truncated"] == true {
        limitations.push("REQUEST_URL_TRUNCATED".into());
    }
    for (side, h) in [("REQUEST", &req["headers"]), ("RESPONSE", &res["headers"])] {
        if h["state"] != "complete" {
            limitations.push(format!(
                "{side}_HEADERS_{}",
                h["state"].as_str().unwrap_or("unknown").to_uppercase()
            ));
        }
    }
    limitations.sort();
    limitations.dedup();
    Ok(ObservedDefinition {
        extractor_version: "observed-http-3".into(),
        schema_view: None,
        method,
        path: url.path().into(),
        request,
        response,
        limitations,
    })
}
/// A marked read projection; persisted revision bytes and identities stay unchanged.
pub fn compact_definition(mut definition: Value) -> Value {
    let mut changed = false;
    for side in ["request", "response"] {
        if let Some(schema) = definition.pointer_mut(&format!("/{side}/body/observed_schema")) {
            let compact = nexofolio_contracts::compact_observed_schema(schema);
            changed |= compact != *schema;
            *schema = compact;
        }
    }
    if changed {
        definition["schema_view"] = json!(nexofolio_contracts::STRUCTURE_ALGORITHM);
    }
    definition
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
fn headers(h: &Value) -> Vec<Value> {
    let names: std::collections::BTreeSet<_> = h["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p[0].as_str().map(str::to_ascii_lowercase))
        .collect();
    names
        .into_iter()
        .map(|name| json!({"name":name,"observed_schema":{"type":"string"}}))
        .collect()
}
fn body(b: &Value, h: &Value, side: &str, notes: &mut Vec<String>) -> Value {
    let state = b["state"].as_str().unwrap_or("unknown");
    let media = media(h);
    if state == "none" {
        return json!({"state":"none","media_type":media});
    }
    if state != "complete" {
        notes.push(format!(
            "{}_BODY_{}",
            side.to_uppercase(),
            state.to_uppercase()
        ));
        return json!({"state":state,"media_type":media,"observed_schema":null});
    }
    let content = b["content"].as_str().unwrap_or("");
    use base64::Engine;
    let decoded = match b["encoding"].as_str() {
        Some("text") => Some(content.as_bytes().to_vec()),
        Some("base64") => base64::engine::general_purpose::STANDARD
            .decode(content)
            .ok(),
        _ => None,
    };
    let schema = if media == "application/json" || media.ends_with("+json") {
        match decoded.and_then(|v| serde_json::from_slice::<Value>(&v).ok()) {
            Some(v) => {
                let observed = nexofolio_contracts::observe_json(&v);
                notes.extend(observed.limitations);
                observed.schema
            }
            None => {
                notes.push(format!("{}_JSON_UNREADABLE", side.to_uppercase()));
                Value::Null
            }
        }
    } else {
        notes.push(format!(
            "{}_NON_JSON_BODY_NOT_STRUCTURALLY_INFERRED",
            side.to_uppercase()
        ));
        Value::Null
    };
    json!({"state":state,"media_type":media,"observed_schema":schema})
}
/// Compatibility helper delegates to the one detailed assessment policy.
pub fn definition_covers(known: &Value, incoming: &Value) -> bool {
    crate::assess_definition(Some(known), incoming).is_duplicate()
}

/// The pinned decision belongs to admission, not to whichever policy is current at processing time.
pub fn apply_observed_path(
    definition: &mut ObservedDefinition,
    path: &nexofolio_contracts::PathIdentity,
) -> Result<()> {
    if path.rule_version != nexofolio_contracts::PATH_RULE_VERSION {
        return Err(nexofolio_contracts::Error::invalid("UNKNOWN_PATH_RULE"));
    }
    if path.parameters.is_empty() {
        return Ok(());
    }
    definition.path = path.template.clone();
    let parameters = definition.request["parameters"]
        .as_array_mut()
        .ok_or(nexofolio_contracts::Error::invalid("INVALID_PARAMETERS"))?;
    for parameter in &path.parameters {
        parameters
            .push(json!({"name":parameter.name,"in":"path","observed_schema":{"type":"string"}}));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw() -> Value {
        let b: Value = serde_json::from_str(include_str!(
            "../../../contracts/ingestion/fixtures/http-batch.json"
        ))
        .unwrap();
        b["records"][0].clone()
    }
    #[test]
    fn compact_read_view_marks_representation_and_preserves_original() {
        let mut original = serde_json::to_value(extract_observed(&raw()).unwrap()).unwrap();
        original["extractor_version"] = json!("observed-http-2");
        original["response"]["body"]["observed_schema"] = json!({"type":"array","items":{"anyOf":[{"type":"object","properties":{"id":{"type":"number"},"name":{"type":"null"}}},{"type":"object","properties":{"id":{"type":"number"},"name":{"type":"string"}}}]}});
        let view = compact_definition(original.clone());
        assert_eq!(view["schema_view"], "http-structure-4");
        assert_eq!(view["extractor_version"], original["extractor_version"]);
        assert_eq!(
            view["response"]["body"]["observed_schema"]["items"]["type"],
            "object"
        );
        assert!(original.get("schema_view").is_none());
        assert!(
            original["response"]["body"]["observed_schema"]["items"]
                .get("anyOf")
                .is_some()
        );
        assert_eq!(compact_definition(view.clone()), view);
        assert!(definition_covers(&original, &view));
    }
    #[test]
    fn empty_array_is_weaker_evidence_not_a_removed_element_schema() {
        let mut raw = raw();
        let full = serde_json::to_value(extract_observed(&raw).unwrap()).unwrap();
        raw["payload"]["response"]["body"]["content"] = json!(r#"{"items":[]}"#);
        let empty = serde_json::to_value(extract_observed(&raw).unwrap()).unwrap();
        assert!(definition_covers(&full, &empty));
        assert!(!definition_covers(&empty, &full));
        raw["payload"]["response"]["body"]["content"] = json!(r#"{"items":[],"new":true}"#);
        assert!(!definition_covers(
            &full,
            &serde_json::to_value(extract_observed(&raw).unwrap()).unwrap()
        ));
        let mut limited = empty.clone();
        limited["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!("STRUCTURE_EXTRACTION_LIMIT"));
        assert!(!definition_covers(&full, &limited));
        let mut limited_full = full.clone();
        limited_full["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!("STRUCTURE_EXTRACTION_LIMIT"));
        assert!(!definition_covers(&limited_full, &limited));
    }
    #[test]
    fn extracts_types_without_required_or_business_guessing() {
        let d = extract_observed(&raw()).unwrap();
        assert_eq!(d.method, "GET");
        assert_eq!(d.path, "/orders");
        assert_eq!(
            d.response["body"]["observed_schema"]["properties"]["items"]["items"]["properties"]["id"]
                ["type"],
            "number"
        );
        assert!(!serde_json::to_string(&d).unwrap().contains("required"));
    }
    #[test]
    fn values_do_not_change_definition_and_raw_is_untouched() {
        let a = raw();
        let mut b = a.clone();
        b["payload"]["response"]["body"]["content"] =
            json!("{\"items\":[{\"name\":\"changed\",\"id\":9},{\"id\":33,\"name\":\"other\"}]}");
        assert_eq!(
            serde_json::to_value(extract_observed(&a).unwrap()).unwrap(),
            serde_json::to_value(extract_observed(&b).unwrap()).unwrap()
        );
        assert!(
            a["payload"]["request"]["headers"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h[0] == "Authorization")
        );
    }
    #[test]
    fn incomplete_null_empty_array_and_bad_json_are_explicit() {
        for content in ["null", "[]", "not json"] {
            let mut a = raw();
            a["payload"]["response"]["body"]["content"] = json!(content);
            assert!(extract_observed(&a).unwrap().limitations.len() > 1);
        }
        let mut a = raw();
        a["payload"]["response"]["body"]["state"] = json!("truncated");
        let d = extract_observed(&a).unwrap();
        assert_eq!(d.response["body"]["observed_schema"], Value::Null);
        assert!(d.limitations.contains(&"RESPONSE_BODY_TRUNCATED".into()));
    }
}
