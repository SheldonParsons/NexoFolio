//! Directional comparison of observed JSON schemas, not validation of API contracts.
use serde_json::Value;

/// Whether new observed schema adds no information/conflict to the known schema.
/// Unknown is a wildcard ONLY in array item position (an actually empty array).
/// Missing object fields and null/type changes stay conservative.
pub fn observed_schema_covers(known: &Value, incoming: &Value) -> bool {
    covers(known, incoming, false)
}
fn covers(known: &Value, incoming: &Value, array_item: bool) -> bool {
    if known == incoming {
        return true;
    }
    if array_item
        && incoming
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.get("unknown") == Some(&Value::Bool(true)))
    {
        return true;
    }
    if let Some(options) = incoming.get("anyOf").and_then(Value::as_array) {
        return !options.is_empty()
            && options
                .iter()
                .all(|option| covers(known, option, array_item));
    }
    if let Some(options) = known.get("anyOf").and_then(Value::as_array) {
        return options
            .iter()
            .any(|option| covers(option, incoming, array_item));
    }
    if known.get("type") != incoming.get("type") {
        return false;
    }
    match incoming.get("type").and_then(Value::as_str) {
        Some("array") => {
            let (Some(a), Some(b)) = (known.get("items"), incoming.get("items")) else {
                return false;
            };
            let (Some(ka), Some(kb)) = (known.as_object(), incoming.as_object()) else {
                return false;
            };
            ka.keys().eq(kb.keys())
                && ka
                    .iter()
                    .all(|(key, v)| key == "items" || Some(v) == kb.get(key))
                && covers(a, b, true)
        }
        Some("object") => {
            let (Some(a), Some(b)) = (
                known.get("properties").and_then(Value::as_object),
                incoming.get("properties").and_then(Value::as_object),
            ) else {
                return false;
            };
            let (Some(ka), Some(kb)) = (known.as_object(), incoming.as_object()) else {
                return false;
            };
            ka.keys().eq(kb.keys())
                && ka
                    .iter()
                    .all(|(key, v)| key == "properties" || Some(v) == kb.get(key))
                && a.keys().eq(b.keys())
                && a.iter().all(|(key, v)| covers(v, &b[key], false))
        }
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn populated_to_empty_is_covered_not_the_reverse() {
        let full =
            json!({"type":"array","items":{"type":"object","properties":{"id":{"type":"number"}}}});
        let empty = json!({"type":"array","items":{"unknown":true}});
        assert!(observed_schema_covers(&full, &empty));
        assert!(!observed_schema_covers(&empty, &full));
        assert!(!observed_schema_covers(&full, &json!({"type":"string"})));
    }
    #[test]
    fn empty_sibling_cannot_hide_field_or_type_changes() {
        let full = json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"number"}},"status":{"type":"string"}}});
        let mut next = full.clone();
        next["properties"]["items"]["items"] = json!({"unknown":true});
        assert!(observed_schema_covers(&full, &next));
        next["properties"]["status"] = json!({"type":"number"});
        assert!(!observed_schema_covers(&full, &next));
        next["properties"].as_object_mut().unwrap().remove("status");
        assert!(!observed_schema_covers(&full, &next));
    }
    #[test]
    fn all_array_variants_are_checked() {
        let known = json!({"type":"array","items":{"anyOf":[{"type":"number"},{"type":"string"}]}});
        assert!(observed_schema_covers(
            &known,
            &json!({"type":"array","items":{"type":"number"}})
        ));
        assert!(!observed_schema_covers(
            &known,
            &json!({"type":"array","items":{"anyOf":[{"type":"number"},{"type":"boolean"}]}})
        ));
        assert!(!observed_schema_covers(
            &json!({"type":"number"}),
            &json!({"unknown":true})
        ));
    }
}
