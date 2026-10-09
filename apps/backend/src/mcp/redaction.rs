//! Defense in depth for old knowledge and every nested layer of saved meanings.
use serde_json::Value;

pub(super) fn redact(value: &mut Value) {
    visit(value, false);
}

fn visit(value: &mut Value, inherited_sensitive: bool) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|v| visit(v, inherited_sensitive)),
        Value::Object(object) => {
            // Search results use method + path for an API route, not a field
            // identity. /oauth/token must retain its harmless name/purpose.
            let api_route = object.get("method").is_some_and(Value::is_string);
            let secret = inherited_sensitive
                || [
                    "path",
                    "field",
                    "field_path",
                    "key",
                    "from_field",
                    "to_field",
                    "pointer",
                ]
                .iter()
                .filter(|key| !(**key == "path" && api_route))
                .filter_map(|key| object.get(*key))
                .any(sensitive_path);
            for (key, value) in object.iter_mut() {
                if sensitive(key) && !metadata(key) {
                    mask(value);
                } else if secret && content(key) {
                    // Evidence carries both payload and source identities. Keep its
                    // objects/arrays and references intact while masking content.
                    mask_content(value);
                } else {
                    visit(value, secret);
                }
            }
        }
        Value::String(text) => {
            if text.to_ascii_lowercase().contains("bearer ")
                || text.starts_with("eyJ")
                || text.starts_with("nfi_")
            {
                *text = "[REDACTED]".into();
            }
            if text.chars().count() > 2000 {
                *text = text.chars().take(2000).collect();
            }
        }
        _ => {}
    }
}

fn sensitive_path(value: &Value) -> bool {
    match value {
        Value::String(path) => sensitive(path),
        Value::Array(parts) => parts.iter().any(sensitive_path),
        Value::Object(parts) => parts.values().any(sensitive_path),
        _ => false,
    }
}

fn content(key: &str) -> bool {
    [
        "text",
        "values",
        "options",
        "details",
        "unknowns",
        "example",
        "description",
        "purpose",
        "name",
        "display_name",
        "unit",
        "role",
        "label",
        "labels",
        "value",
        "evidence",
        "condition",
        "scope",
        "page",
    ]
    .contains(&key)
}

fn metadata(key: &str) -> bool {
    [
        "credential_ref",
        "authentication",
        "authenticated",
        "path",
        "field",
        "field_path",
        "key",
        "from_field",
        "to_field",
        "pointer",
        "endpoint",
        "endpoint_id",
        "environment",
        "environment_id",
        "id",
        "from",
        "to",
        "relation",
        "location",
        "from_location",
        "to_location",
        "in",
        "status",
        "kind",
        "confirmation",
        "unit_confirmation",
        "display_name_confirmation",
        "carrier",
        "scheme",
        "recipe",
        "input_hash",
        "at",
        "sequence",
        "complete",
        "platform",
    ]
    .contains(&key)
}

fn mask_content(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(mask_content),
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if metadata(key) {
                    visit(value, false);
                } else {
                    mask_content(value);
                }
            }
        }
        _ => mask(value),
    }
}

/// Keep JSON containers intact, including arrays consumed by MCP clients.
fn mask(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(mask),
        Value::Object(object) => object.values_mut().for_each(mask),
        Value::Null => {}
        _ => *value = Value::String("[REDACTED]".into()),
    }
}

fn sensitive(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "authorization",
        "cookie",
        "phone",
        "mobile",
        "email",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sensitive_ancestry_covers_meanings_and_both_relation_ends_without_breaking_arrays() {
        let dummy = "DUMMY_REVIEW_ONLY";
        let mut output = serde_json::json!({
            "knowledge":{"path":"request_body:password","text":dummy,"values":[dummy],
                "context":{"display_name":dummy,"meaning":{"unit":dummy,"role":dummy,"details":[dummy],"unknowns":[dummy]},
                    "evidence":[{"kind":"observation","field":"password","endpoint":"reference-id","options":[{"value":dummy,"label":dummy}],"source":{"pointer":"/request/password","environment":"env-id"}}]}},
            "links":[{"from_field":"access_token","evidence":dummy,"context":{"condition":dummy}},
                {"to_field":"password","evidence":dummy,"context":{"sources":[{"example":dummy,"pointer":"/password"}]}}]
        });
        redact(&mut output);
        assert!(!output.to_string().contains(dummy));
        assert!(output["knowledge"]["values"].is_array());
        assert!(output["knowledge"]["context"]["meaning"]["details"].is_array());
        assert!(output["knowledge"]["context"]["evidence"].is_array());
        assert_eq!(
            output["knowledge"]["context"]["evidence"][0]["endpoint"],
            "reference-id"
        );
        assert_eq!(
            output["knowledge"]["context"]["evidence"][0]["source"]["pointer"],
            "/request/password"
        );
        assert_eq!(output["links"][0]["from_field"], "access_token");
    }
    #[test]
    fn normal_knowledge_remains_readable_and_fact_paths_also_protect_labels() {
        let normal = serde_json::json!({"path":"amount","context":{"display_name":"金额","meaning":{"unit":"万","details":["预计金额"],"unknowns":["币种未确认"]}}});
        let mut output = normal.clone();
        redact(&mut output);
        assert_eq!(normal, output);
        let route = serde_json::json!({"method":"POST","path":"/oauth/token","name":"获取业务认证令牌","purpose":"使用当前认证信息登录"});
        let mut output = route.clone();
        redact(&mut output);
        assert_eq!(
            route, output,
            "API route names are not captured credentials"
        );
        let mut fact =
            serde_json::json!({"path":[{"key":"password"}],"labels":["DUMMY_REVIEW_ONLY"]});
        redact(&mut fact);
        assert!(!fact.to_string().contains("DUMMY_REVIEW_ONLY"));
        assert!(fact["labels"].is_array());
    }
    #[test]
    fn old_sensitive_field_notes_and_bearer_strings_are_redacted() {
        let mut value = serde_json::json!({"fields":[{"key":"request_body:password","knowledge":{
            "path":"request_body:password","text":"historical-password","values":["historical-password"],
            "context":{"evidence":[{"field":"password","options":[{"value":"historical-password"}]}]}}}],
            "note":"Bearer historical-token","authentication":[{"carrier":"Authorization","scheme":"Bearer","credential_ref":"redacted:sample:request.authorization"}]});
        redact(&mut value);
        let text = value.to_string();
        assert!(!text.contains("historical-password"));
        assert!(!text.contains("historical-token"));
        assert!(text.contains("redacted:sample:request.authorization"));
    }
}
