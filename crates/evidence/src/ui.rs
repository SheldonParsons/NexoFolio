use serde_json::Value;

/// A snapshot supplies several controls, whereas an interaction supplies its target.
/// Equal-valued controls and duplicate element IDs cannot establish a unique binding.
/// Unknown, hidden, or ambiguous controls remain available in the original evidence.
pub fn unambiguous_ui_targets(payload: &Value) -> Vec<&Value> {
    let targets: Vec<_> = match payload.get("elements").and_then(Value::as_array) {
        Some(elements) => elements.iter().collect(),
        None => payload.get("target").into_iter().collect(),
    };
    let mut ids = std::collections::HashMap::new();
    let mut values = std::collections::HashMap::new();
    for target in &targets {
        if let Some(id) = target["element_id"].as_str() {
            *ids.entry(id).or_insert(0) += 1;
        }
        if target["visible"] == true && target["value"]["state"] == "present" {
            *values
                .entry(value_key(&target["value"]["value"]))
                .or_insert(0) += 1;
        }
    }
    targets
        .into_iter()
        .filter(|target| {
            target["visible"] == true
                && target["element_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && ids.get(id) == Some(&1))
                && target["value"]["state"] != "unknown"
                && (target["value"]["state"] != "present"
                    || values.get(&value_key(&target["value"]["value"])) == Some(&1))
        })
        .collect()
}

// Same equivalence as value_match; booleans/null never collapse into strings.
fn value_key(value: &Value) -> String {
    match value {
        Value::String(s) => serde_json::json!(["scalar", s]).to_string(),
        Value::Number(n) => serde_json::json!(["scalar", n.to_string()]).to_string(),
        other => serde_json::json!(["typed", other]).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_ambiguity_and_unknown_values_are_not_bindings() {
        let element = |id, value| json!({"element_id":id,"visible":true,"value":{"state":"present","value":value}});
        let mut unknown = element("unknown", Value::Null);
        unknown["value"]["state"] = json!("unknown");
        let mut hidden = element("hidden", json!("confirmed"));
        hidden["visible"] = json!(false);
        let payload = json!({"elements":[
            element("status", json!("1")), element("type", json!(1)),
            element("duplicate-id", json!("a")), element("duplicate-id", json!("b")),
            unknown, hidden, element("unique", json!("confirmed")),
            element("null", Value::Null), element("boolean", json!(true))
        ]});
        let ids: Vec<_> = unambiguous_ui_targets(&payload)
            .into_iter()
            .map(|target| target["element_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["unique", "null", "boolean"]);
        assert_eq!(
            unambiguous_ui_targets(&json!({"target":element("status", json!("1"))})).len(),
            1
        );
    }
}
