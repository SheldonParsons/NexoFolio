use nexofolio_contracts::*;
use serde_json::{Value, json};
#[derive(Debug, Clone)]
pub struct ValueOccurrence {
    pub field: FieldRef,
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
#[derive(Debug, Default)]
pub struct Extraction {
    pub values: Vec<ValueOccurrence>,
    pub facts: Vec<FactDraft>,
    pub limitations: Vec<String>,
}
/// Field paths escape JSON Pointer tokens; '*' marks array item schema paths only.
fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
fn walk(v: &Value, field: &FieldRef, pointer: &str, out: &mut Extraction, budget: &mut usize) {
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
    let mut out = Extraction::default();
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
        if let Some(body) = decode_body(&payload[side]["body"]) {
            let mut f = base.clone();
            f.location = format!("{side}.body");
            f.path = "".into();
            walk(&body, &f, "", &mut out, &mut budget);
        } else if payload[side]["body"]["state"] != "none" {
            out.limitations.push(format!(
                "{}_BODY_NOT_JSON_OR_INCOMPLETE",
                side.to_uppercase()
            ));
        }
    }
    for side in ["request", "response"] {
        for (index, entry) in payload[side]["headers"]["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let (Some(name), Some(value)) = (entry[0].as_str(), entry[1].as_str()) {
                let mut field = base.clone();
                field.location = format!("{side}.header");
                field.path = format!("/{}", escape(&name.to_ascii_lowercase()));
                walk(
                    &Value::String(value.into()),
                    &field,
                    &format!("/headers/entries/{index}/1"),
                    &mut out,
                    &mut budget,
                );
            }
        }
    }
    out
}
/// Dictionary patterns remain observed mappings, never complete API enum constraints.
pub fn dictionary_facts(payload: &Value, values: &[ValueOccurrence]) -> Vec<FactDraft> {
    let url = payload["request"]["url"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
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
                out.push(FactDraft{kind:"dictionary_mapping_candidate".into(),subject:serde_json::to_value(&value.field).unwrap(),data:json!({"state":"present","value":value.value,"label":other.value,"complete_enum":false,"request_url":payload["request"]["url"],"verification":"observed_mapping"})});
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
pub fn relation_field(field: &FieldRef) -> bool {
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
        return false;
    }
    ![
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
    .any(|key| leaf == *key || leaf.ends_with(key))
}

/// Absence requires a complete readable request. Missing/truncated capture never proves omission.
pub fn field_absent(payload: &Value, field: &FieldRef) -> bool {
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
        assert!(field_absent(&payload("complete", "{}"), &f));
        assert!(!field_absent(&payload("complete", "{\"status\":null}"), &f));
        assert!(!field_absent(&payload("truncated", "{}"), &f));
        assert!(!field_absent(&payload("complete", "not json"), &f));
    }
    #[test]
    fn credentials_are_excluded_from_correlations_without_redaction() {
        let mut f = field();
        f.path = "/access_token".into();
        assert!(!relation_field(&f));
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
