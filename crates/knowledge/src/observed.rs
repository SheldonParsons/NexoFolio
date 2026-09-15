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
    let mut budget = 10000usize;
    let query: std::collections::BTreeSet<_> = url
        .query_pairs()
        .map(|(name, _)| name.to_string())
        .collect();
    let params: Vec<_> = query
        .into_iter()
        .map(|name| json!({"name":name,"in":"query","observed_schema":{"type":"string"}}))
        .collect();
    let request = json!({"parameters":params,"headers":headers(&req["headers"]),"body":body(&req["body"],&req["headers"],"request",&mut budget,&mut limitations)});
    let response = json!({"status":res["status"],"capture_state":res["state"],"headers":headers(&res["headers"]),"body":body(&res["body"],&res["headers"],"response",&mut budget,&mut limitations)});
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
        extractor_version: "observed-http-1".into(),
        method,
        path: url.path().into(),
        request,
        response,
        limitations,
    })
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
fn body(b: &Value, h: &Value, side: &str, budget: &mut usize, notes: &mut Vec<String>) -> Value {
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
            Some(v) => shape(&v, 0, budget, notes),
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
fn shape(v: &Value, depth: usize, budget: &mut usize, notes: &mut Vec<String>) -> Value {
    if *budget == 0 || depth > 64 {
        notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
        return json!({"unknown":true});
    }
    *budget -= 1;
    match v {
        Value::Null => {
            notes.push("NULL_DOES_NOT_ESTABLISH_FIELD_TYPE".into());
            json!({"type":"null"})
        }
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Number(_) => json!({"type":"number"}),
        Value::String(_) => json!({"type":"string"}),
        Value::Object(map) => {
            let mut props = serde_json::Map::new();
            for (k, v) in map {
                if *budget == 0 {
                    notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
                    break;
                }
                props.insert(k.clone(), shape(v, depth + 1, budget, notes));
            }
            json!({"type":"object","properties":props})
        }
        Value::Array(items) => {
            let mut shapes = std::collections::BTreeMap::new();
            if items.is_empty() {
                notes.push("EMPTY_ARRAY_ITEM_TYPE_UNKNOWN".into());
            }
            for item in items {
                if *budget == 0 {
                    notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
                    break;
                }
                let s = shape(item, depth + 1, budget, notes);
                shapes.insert(serde_json::to_string(&s).unwrap(), s);
            }
            let distinct: Vec<Value> = shapes.into_values().collect();
            json!({"type":"array","items":if distinct.len()==1{distinct[0].clone()}else if distinct.is_empty(){json!({"unknown":true})}else{json!({"anyOf":distinct})}})
        }
    }
}

/// Ignore only loss of array item information; all other metadata and fields are checked.
pub fn definition_covers(known: &Value, incoming: &Value) -> bool {
    if known == incoming {
        return true;
    }
    // Budget exhaustion also emits unknown markers. It must never act as empty-array evidence.
    if [known, incoming].iter().any(|definition| {
        definition["limitations"].as_array().is_some_and(|notes| {
            notes
                .iter()
                .any(|note| note == "STRUCTURE_EXTRACTION_LIMIT")
        })
    }) {
        return false;
    }
    let (Some(a), Some(b)) = (known.as_object(), incoming.as_object()) else {
        return false;
    };
    if !a.keys().eq(b.keys()) {
        return false;
    }
    for (key, value) in a {
        if key == "limitations" {
            let filtered = |v: &Value| -> Option<Vec<String>> {
                v.as_array().map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|s| *s != "EMPTY_ARRAY_ITEM_TYPE_UNKNOWN")
                        .map(str::to_owned)
                        .collect()
                })
            };
            if filtered(value) != filtered(&b[key]) {
                return false;
            }
        } else if key == "request" || key == "response" {
            let (Some(x), Some(y)) = (value.as_object(), b[key].as_object()) else {
                return false;
            };
            if !x.keys().eq(y.keys()) {
                return false;
            }
            for (field, v) in x {
                if field != "body" {
                    if Some(v) != y.get(field) {
                        return false;
                    }
                    continue;
                }
                let (Some(xb), Some(yb)) = (v.as_object(), y[field].as_object()) else {
                    return false;
                };
                if !xb.keys().eq(yb.keys()) {
                    return false;
                }
                for (f, v) in xb {
                    if f == "observed_schema" {
                        if !nexofolio_contracts::observed_schema_covers(v, &yb[f]) {
                            return false;
                        }
                    } else if Some(v) != yb.get(f) {
                        return false;
                    }
                }
            }
        } else if Some(value) != b.get(key) {
            return false;
        }
    }
    true
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
