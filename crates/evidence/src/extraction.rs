use nexofolio_contracts::*;
use serde_json::{Value, json};
#[derive(Debug, Clone)]
pub struct ValueOccurrence {
    pub field: EvidenceFieldRef,
    pub pointer: String,
    pub value: Value,
    pub direction: &'static str,
}
#[derive(Debug, Clone)]
pub struct FactDraft {
    pub kind: String,
    pub subject: Value,
    pub data: Value,
}
/// A bounded set of observed values, never an inferred complete enum.
/// Full occurrences still feed short-lived relation/UI indexes independently.
pub const OBSERVED_VALUE_LIMIT: i64 = 16;
pub fn observed_value_limit(subject: Value) -> FactDraft {
    FactDraft {
        kind: "observed_value_limit".into(),
        subject,
        data: json!({"retained_value_limit":OBSERVED_VALUE_LIMIT,"complete_enum":false,"reason":"VALUE_SAMPLE_LIMIT","policy_version":"value-samples-1"}),
    }
}
#[derive(Debug, Default)]
pub struct Extraction {
    pub values: Vec<ValueOccurrence>,
    pub facts: Vec<FactDraft>,
    pub limitations: Vec<String>,
    pub request_complete: bool,
    pub response_complete: bool,
}
/// Field paths escape JSON Pointer tokens; '*' marks array item schema paths only.
fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
fn walk(
    v: &Value,
    field: &EvidenceFieldRef,
    pointer: &str,
    out: &mut Extraction,
    budget: &mut usize,
) {
    if *budget == 0 {
        if !out
            .limitations
            .iter()
            .any(|s| s == "EVIDENCE_EXTRACTION_LIMIT")
        {
            out.limitations.push("EVIDENCE_EXTRACTION_LIMIT".into());
        }
        return;
    }
    *budget -= 1;
    match v {
        Value::Object(o) => {
            for (k, v) in o {
                let mut child = field.clone();
                child.path = format!("{}/{}", field.path, escape(k));
                walk(v, &child, &format!("{pointer}/{}", escape(k)), out, budget);
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                let mut child = field.clone();
                child.path = format!("{}/*", field.path);
                walk(v, &child, &format!("{pointer}/{i}"), out, budget);
            }
        }
        _ => {
            out.values.push(ValueOccurrence {
                field: field.clone(),
                pointer: pointer.into(),
                value: v.clone(),
                direction: if field.location.starts_with("response") {
                    "response"
                } else {
                    "request"
                },
            });
            out.facts.push(FactDraft {
                kind: "observed_value".into(),
                subject: serde_json::to_value(field).unwrap(),
                data: json!({"state":"present","value":v,"complete_enum":false}),
            });
        }
    }
}
fn decode_body(body: &Value) -> Option<Value> {
    if body["state"] != "complete" {
        return None;
    }
    let text = body["content"].as_str()?;
    if body["encoding"] == "base64" {
        use base64::Engine;
        serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(text)
                .ok()?,
        )
        .ok()
    } else {
        serde_json::from_str(text).ok()
    }
}
pub fn extract_http(payload: &Value, base: FieldRef, path: Option<&PathIdentity>) -> Extraction {
    let mut out = Extraction {
        request_complete: payload["request"]["url_truncated"] != true,
        response_complete: true,
        ..Extraction::default()
    };
    let base: EvidenceFieldRef = base.into();
    let mut budget = 512;
    if let Some(url) = payload["request"]["url"]
        .as_str()
        .and_then(|s| url::Url::parse(s).ok())
    {
        for (k, v) in url.query_pairs() {
            let mut f = base.clone();
            f.location = "request.query".into();
            f.path = format!("/{}", escape(&k));
            walk(
                &Value::String(v.into_owned()),
                &f,
                &f.path.clone(),
                &mut out,
                &mut budget,
            );
        }
        let identity = path.cloned().unwrap_or(PathIdentity {
            rule_version: PATH_RULE_VERSION.into(),
            template: url.path().into(),
            parameters: vec![],
        });
        let parts: Vec<_> = url.path().split('/').collect();
        for p in identity.parameters {
            let mut f = base.clone();
            f.location = "request.path".into();
            f.path = format!("/{}", p.name);
            walk(
                &Value::String(parts[p.segment_index].into()),
                &f,
                &format!("/{}", p.segment_index),
                &mut out,
                &mut budget,
            );
        }
    }
    for side in ["request", "response"] {
        if side == "response" {
            budget = 512;
        }
        let mut complete = if side == "request" {
            out.request_complete
        } else {
            true
        };
        if let Some(body) = decode_body(&payload[side]["body"]) {
            let mut f = base.clone();
            f.location = format!("{side}.body");
            f.path.clear();
            walk(&body, &f, "", &mut out, &mut budget);
        } else if payload[side]["body"]["state"] != "none" {
            complete = false;
            out.limitations.push(format!(
                "{}_BODY_NOT_JSON_OR_INCOMPLETE",
                side.to_uppercase()
            ));
        }
        for (index, entry) in payload[side]["headers"]["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let (Some(name), Some(value)) = (entry[0].as_str(), entry[1].as_str()) {
                let mut f = base.clone();
                f.location = format!("{side}.header");
                f.path = format!("/{}", escape(&name.to_ascii_lowercase()));
                walk(
                    &Value::String(value.into()),
                    &f,
                    &format!("/headers/entries/{index}/1"),
                    &mut out,
                    &mut budget,
                );
            }
        }
        complete &= budget > 0;
        if side == "request" {
            out.request_complete = complete;
        } else {
            out.response_complete = complete;
        }
    }
    out
}
/// Dictionary patterns remain observed mappings, never complete API enum constraints.
pub fn dictionary_facts(payload: &Value, values: &[ValueOccurrence]) -> Vec<FactDraft> {
    let url = payload["request"]["url"]
        .as_str()
        .and_then(|s| url::Url::parse(s).ok())
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_default();
    if !["dict", "enum", "option"]
        .iter()
        .any(|part| url.contains(part))
    {
        return vec![];
    }
    let mut out = Vec::new();
    for value in values.iter().filter(|v| v.direction == "response") {
        let Some((parent, key)) = value.pointer.rsplit_once('/') else {
            continue;
        };
        let labels = match key {
            "value" => ["label", "name"],
            "code" => ["name", "label"],
            "id" => ["name", "label"],
            _ => continue,
        };
        for label in labels {
            if let Some(other) = values.iter().find(|v| {
                v.direction == "response"
                    && v.pointer == format!("{parent}/{label}")
                    && v.value.is_string()
            }) {
                out.push(FactDraft{kind:"dictionary_mapping_candidate".into(),subject:serde_json::to_value(&value.field).unwrap(),data:json!({"state":"present","value":value.value,"label":other.value,"complete_enum":false,"request_url":payload["request"]["url"],"scope":request_scope(payload),"verification":"observed_mapping"})});
                break;
            }
        }
    }
    out
}

pub fn ui_facts(
    kind: &CaptureKind,
    payload: &Value,
    context: &Option<CaptureContext>,
) -> Vec<FactDraft> {
    let subject = json!({"context":context.as_ref().map(|c|json!({"page_url":c.page_url,"source":"page"})),"source_kind":kind});
    match kind {
        CaptureKind::PageContext => vec![FactDraft {
            kind: "page_context".into(),
            subject,
            data: payload.clone(),
        }],
        CaptureKind::Interaction => vec![FactDraft {
            kind: "ui_interaction".into(),
            subject,
            data: payload.clone(),
        }],
        CaptureKind::UiSnapshot => vec![FactDraft {
            kind: "ui_options".into(),
            subject,
            data: payload.clone(),
        }],
        CaptureKind::ImageReference => vec![FactDraft {
            kind: "page_image".into(),
            subject,
            data: payload.clone(),
        }],
        _ => vec![],
    }
}
/// String/number equivalence is recorded as an explicit conversion, never null/boolean coercion.
pub fn value_match(source: &Value, target: &Value) -> Option<&'static str> {
    if source == target {
        return Some("identity");
    }
    if source.is_number() && target.as_str() == Some(source.to_string().as_str()) {
        return Some("number_to_string");
    }
    if target.is_number() && source.as_str() == Some(target.to_string().as_str()) {
        return Some("string_to_number");
    }
    None
}
pub fn informative(value: &Value) -> bool {
    match value {
        Value::String(s) => s.len() > 1 && !matches!(s.as_str(), "0" | "1" | "true" | "false"),
        Value::Number(n) => n.as_i64().is_none_or(|n| n != 0 && n != 1),
        _ => false,
    }
}

/// Technical credentials remain stored as raw evidence but never create value-equality relations.
pub fn relation_exclusion(field: &EvidenceFieldRef) -> Option<&'static str> {
    let leaf: String = field
        .path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if field.location.ends_with(".header")
        && [
            "contenttype",
            "contentlength",
            "date",
            "server",
            "cachecontrol",
            "etag",
            "lastmodified",
            "vary",
            "accept",
            "acceptencoding",
            "useragent",
            "connection",
            "xrequestid",
            "xcorrelationid",
            "traceparent",
            "tracestate",
        ]
        .contains(&leaf.as_str())
    {
        return Some("protocol_header");
    }
    if [
        "sort",
        "size",
        "pagesize",
        "pageno",
        "pageindex",
        "page",
        "offset",
        "limit",
        "width",
        "height",
        "templateimagewidth",
        "templateimageheight",
        "x",
        "y",
    ]
    .contains(&leaf.as_str())
    {
        return Some("low_information_field");
    }
    if [
        "token",
        "authorization",
        "password",
        "passwd",
        "secret",
        "cookie",
        "session",
        "sessionid",
        "csrf",
        "signature",
        "apikey",
    ]
    .iter()
    .any(|key| leaf == *key || leaf.ends_with(key) || leaf == format!("{key}value"))
    {
        Some("technical_field")
    } else {
        None
    }
}
pub fn relation_field(field: &EvidenceFieldRef) -> bool {
    relation_exclusion(field).is_none()
}

/// Absence requires a complete readable request. Missing/truncated capture never proves omission.
pub fn field_absent(payload: &Value, field: &EvidenceFieldRef) -> bool {
    match field.location.as_str() {
        "request.query" => payload["request"]["url"]
            .as_str()
            .and_then(|s| url::Url::parse(s).ok())
            .is_some_and(|u| {
                !u.query_pairs()
                    .any(|(k, _)| format!("/{}", escape(&k)) == field.path)
            }),
        "request.body" if !field.path.contains("/*") => decode_body(&payload["request"]["body"])
            .is_some_and(|v| v.is_object() && v.pointer(&field.path).is_none()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn field() -> FieldRef {
        FieldRef {
            interface_id: InterfaceId::new(),
            environment_id: EnvironmentId::new(),
            revision_id: RevisionId::new(),
            location: "request.body".into(),
            path: "/status".into(),
        }
    }
    #[test]
    fn values_do_not_coerce_null_omission_or_leading_zero_strings() {
        assert_eq!(
            value_match(&json!(23), &json!("23")),
            Some("number_to_string")
        );
        assert_eq!(
            value_match(&json!("23"), &json!(23)),
            Some("string_to_number")
        );
        for (a, b) in [
            (json!(1), json!("01")),
            (json!(null), json!("null")),
            (json!(true), json!(1)),
        ] {
            assert_eq!(value_match(&a, &b), None);
        }
        for value in [json!(0), json!(1), json!("1"), json!(null), json!(false)] {
            assert!(!informative(&value));
        }
    }
    #[test]
    fn complete_request_is_required_to_claim_field_omission() {
        let f = field();
        let payload = |state: &str, body: &str| json!({"request":{"body":{"state":state,"encoding":"text","content":body}}});
        assert!(field_absent(&payload("complete", "{}"), &f.clone().into()));
        assert!(!field_absent(
            &payload("complete", "{\"status\":null}"),
            &f.clone().into()
        ));
        assert!(!field_absent(
            &payload("truncated", "{}"),
            &f.clone().into()
        ));
        assert!(!field_absent(
            &payload("complete", "not json"),
            &f.clone().into()
        ));
    }
    #[test]
    fn credentials_are_excluded_from_correlations_without_redaction() {
        let mut f = field();
        f.path = "/access_token".into();
        assert!(!relation_field(&f.clone().into()));
        let payload = json!({"request":{"body":{"state":"complete","encoding":"text","content":"{\"access_token\":\"sample-token\"}"}},"response":{"body":{"state":"none"}}});
        let out = extract_http(&payload, f, None);
        assert!(out.values.iter().any(|v| v.value == "sample-token"));
    }
    #[test]
    fn arrays_have_wildcard_fields_and_concrete_sample_locations() {
        let payload = json!({"request":{"body":{"state":"none"}},"response":{"body":{"state":"complete","encoding":"text","content":"{\"items\":[{\"id\":23},{\"id\":24}]}"}}});
        let out = extract_http(&payload, field(), None);
        assert_eq!(out.values[0].field.path, "/items/*/id");
        assert_eq!(out.values[1].pointer, "/items/1/id");
        assert!(out.facts.iter().all(|f| f.data["complete_enum"] == false));
    }
    #[test]
    fn business_headers_are_observed_and_protocol_headers_do_not_create_relations() {
        let payload = json!({"request":{"headers":{"entries":[["X-Tenant-Id","23"],["Content-Type","application/json"],["Authorization","Bearer sample"]]},"body":{"state":"none"}},"response":{"headers":{"entries":[]},"body":{"state":"none"}}});
        let extracted = extract_http(&payload, field(), None);
        let tenant = extracted
            .values
            .iter()
            .find(|v| v.field.path == "/x-tenant-id")
            .unwrap();
        assert_eq!(tenant.field.location, "request.header");
        assert_eq!(tenant.pointer, "/headers/entries/0/1");
        assert!(relation_field(&tenant.field));
        assert!(
            extracted
                .values
                .iter()
                .filter(|v| v.field.path != "/x-tenant-id")
                .all(|v| !relation_field(&v.field))
        );
        assert!(extracted.values.iter().any(|v| v.value == "Bearer sample"));
    }
}

/// Exact observed request conditions. The adapter stores a digest plus sample references,
/// not a copy of the request in every mapping. Incomplete conditions cannot establish a conflict.
pub fn request_scope(payload: &Value) -> Value {
    let url = payload["request"]["url"]
        .as_str()
        .and_then(|s| url::Url::parse(s).ok());
    let body = &payload["request"]["body"];
    let parsed = decode_body(body);
    json!({"complete":url.is_some() && payload["request"]["url_truncated"] != true && (body["state"]=="none" || parsed.is_some()),
        "method":payload["request"]["method"],"url":url.map(|u|u.to_string()),"body":parsed,
        "body_state":body["state"],"headers":payload["request"]["headers"]})
}
impl Extraction {
    /// The compared revision is a possible basis, never automatic ownership of every observed field.
    pub fn bind_source(&mut self, project: ProjectId, ingestion: uuid::Uuid, definition: &Value) {
        for (value, fact) in self.values.iter_mut().zip(&mut self.facts) {
            let f = &mut value.field;
            if !field_covered(definition, f, &value.value) {
                f.revision_id = None;
                f.observation = Some(ObservationFieldRef {
                    project_id: project,
                    interface_id: f.interface_id,
                    environment_id: f.environment_id,
                    ingestion_id: ingestion,
                    location: f.location.clone(),
                    path: f.path.clone(),
                });
            }
            fact.subject = serde_json::to_value(&f).unwrap();
            if let Some(reason) = relation_exclusion(f) {
                fact.data["relation_exclusion"] = json!(reason);
            }
        }
    }
}
fn field_covered(definition: &Value, field: &EvidenceFieldRef, value: &Value) -> bool {
    let Some((side, location)) = field.location.split_once('.') else {
        return false;
    };
    let shape = observe_json(value).schema;
    if location == "body" {
        let tokens: Vec<_> = field
            .path
            .split('/')
            .skip(1)
            .map(|s| s.replace("~1", "/").replace("~0", "~"))
            .collect();
        schema_covers_at(
            &definition[side]["body"]["observed_schema"],
            &tokens,
            &shape,
        )
    } else {
        let (list, key) = if location == "header" {
            ("headers", "name")
        } else {
            ("parameters", "name")
        };
        definition[side][list]
            .as_array()
            .into_iter()
            .flatten()
            .any(|f| {
                (location == "header" || f["in"] == location)
                    && f[key]
                        .as_str()
                        .is_some_and(|name| format!("/{}", escape(name)) == field.path)
                    && observed_schema_covers(&f["observed_schema"], &shape)
            })
    }
}
fn schema_covers_at(schema: &Value, path: &[String], shape: &Value) -> bool {
    if let Some(variants) = schema["anyOf"].as_array() {
        return variants.iter().any(|v| schema_covers_at(v, path, shape));
    }
    if path.is_empty() {
        return observed_schema_covers(schema, shape);
    }
    if schema["type"] == "array" && path[0] == "*" {
        schema_covers_at(&schema["items"], &path[1..], shape)
    } else if let Some(child) = schema["properties"].get(&path[0]) {
        schema_covers_at(child, &path[1..], shape)
    } else {
        false
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    fn base() -> FieldRef {
        FieldRef {
            interface_id: InterfaceId::new(),
            environment_id: EnvironmentId::new(),
            revision_id: RevisionId::new(),
            location: String::new(),
            path: String::new(),
        }
    }
    fn body(value: Value) -> Value {
        json!({"state":"complete","encoding":"text","content":value.to_string()})
    }
    #[test]
    fn unadopted_fields_reference_observation_and_keep_typed_values() {
        let basis = base();
        let revision = basis.revision_id;
        let ingestion = uuid::Uuid::new_v4();
        let payload = json!({"request":{"body":body(json!({"known":1,"new":"1","null":null}))},"response":{"body":{"state":"none"}}});
        let definition = json!({"request":{"body":{"observed_schema":{"type":"object","properties":{"known":{"type":"number"},"null":{"type":"null"}}}}}});
        let mut out = extract_http(&payload, basis, None);
        out.bind_source(ProjectId::new(), ingestion, &definition);
        let new = out.values.iter().find(|v| v.field.path == "/new").unwrap();
        assert!(new.field.revision_id.is_none());
        assert_eq!(
            new.field.observation.as_ref().unwrap().ingestion_id,
            ingestion
        );
        assert_eq!(new.value, json!("1"));
        let known = out
            .values
            .iter()
            .find(|v| v.field.path == "/known")
            .unwrap();
        assert_eq!(known.field.revision_id, Some(revision));
        assert!(known.field.observation.is_none());
        assert!(
            out.facts
                .iter()
                .any(|f| f.subject["observation"]["ingestion_id"] == ingestion.to_string())
        );
    }
    #[test]
    fn coverage_is_directional_and_conditions_preserve_types() {
        let payload = json!({"request":{"body":body(json!({"status":1}))},"response":{"body":body(json!({"items":vec![1;600]}))}});
        let out = extract_http(&payload, base(), None);
        assert!(out.request_complete);
        assert!(!out.response_complete);
        let reversed = json!({"request":payload["response"],"response":payload["request"]});
        let out = extract_http(&reversed, base(), None);
        assert!(!out.request_complete);
        assert!(out.response_complete);
        let mut north = payload.clone();
        north["request"]["url"] = json!("https://test.invalid/dict");
        let mut south = north.clone();
        south["request"]["body"] = body(json!({"status":"1"}));
        assert_ne!(request_scope(&north), request_scope(&south));
        south["request"]["body"]["state"] = json!("truncated");
        assert_eq!(request_scope(&south)["complete"], false);
    }
    #[test]
    fn technical_and_low_information_values_stay_observed_but_not_correlated() {
        let payload = json!({"request":{"body":body(json!({"tokenValue":"unredacted","size":23,"record_id":23}))},"response":{"body":{"state":"none"}}});
        let out = extract_http(&payload, base(), None);
        for v in &out.values {
            assert_eq!(relation_field(&v.field), v.field.path == "/record_id");
        }
        assert!(out.values.iter().any(|v| v.value == "unredacted"));
    }
}
