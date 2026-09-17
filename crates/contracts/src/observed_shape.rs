//! Canonical, bounded JSON observation. Unknown type is distinct from an unread subtree.
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const OBSERVED_NODE_LIMIT: usize = 100_000;
pub const OBSERVED_DEPTH_LIMIT: usize = 64;
pub const STRUCTURE_ALGORITHM: &str = "http-structure-4";

pub struct ObservedShape {
    pub schema: Value,
    pub limitations: Vec<String>,
}
pub fn observe_json(value: &Value) -> ObservedShape {
    let mut notes = vec![];
    let mut remaining = OBSERVED_NODE_LIMIT;
    let schema = shape(value, 0, &mut remaining, &mut notes);
    notes.sort();
    notes.dedup();
    ObservedShape {
        schema,
        limitations: notes,
    }
}
fn shape(value: &Value, depth: usize, remaining: &mut usize, notes: &mut Vec<String>) -> Value {
    if *remaining == 0 || depth > OBSERVED_DEPTH_LIMIT {
        notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
        return json!({"unknown":true,"reason":"extraction_limit"});
    }
    *remaining -= 1;
    match value {
        Value::Null => {
            notes.push("NULL_DOES_NOT_ESTABLISH_FIELD_TYPE".into());
            json!({"type":"null"})
        }
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Number(_) => json!({"type":"number"}),
        Value::String(_) => json!({"type":"string"}),
        Value::Object(values) => {
            let mut props = serde_json::Map::new();
            for (key, value) in values {
                if *remaining == 0 {
                    break;
                }
                props.insert(key.clone(), shape(value, depth + 1, remaining, notes));
            }
            let partial = props.len() != values.len();
            let mut out = json!({"type":"object","properties":props});
            if partial {
                notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
                out["x-observed-incomplete"] = json!(true);
            }
            out
        }
        Value::Array(values) => {
            if values.is_empty() {
                notes.push("EMPTY_ARRAY_ITEM_TYPE_UNKNOWN".into());
            }
            let mut variants = BTreeMap::new();
            let mut visited = 0;
            for value in values {
                if *remaining == 0 {
                    break;
                }
                let s = shape(value, depth + 1, remaining, notes);
                variants.insert(s.to_string(), s);
                visited += 1;
            }
            let mut out =
                json!({"type":"array","items":alternatives(variants.into_values().collect())});
            if visited != values.len() {
                notes.push("STRUCTURE_EXTRACTION_LIMIT".into());
                out["x-observed-incomplete"] = json!(true);
            }
            out
        }
    }
}

/// Field-level observation, not a validator or a claim about joint value combinations.
/// Missing keys and concrete type conflicts retain separate object alternatives.
fn merge(a: &Value, b: &Value, budget: &mut usize) -> Option<Value> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    if a == b {
        return Some(a.clone());
    }
    if a == &json!({"unknown":true}) {
        return Some(b.clone());
    }
    if b == &json!({"unknown":true}) {
        return Some(a.clone());
    }
    let split_null = |v: &Value| -> Option<(Option<Value>, bool)> {
        if v == &json!({"type":"null"}) {
            return Some((None, true));
        }
        if let Some(items) = v.get("anyOf").and_then(Value::as_array) {
            if items.len() == 2 && items.contains(&json!({"type":"null"})) {
                return Some((items.iter().find(|v| v["type"] != "null").cloned(), true));
            }
            return None;
        }
        Some((Some(v.clone()), false))
    };
    let (av, an) = split_null(a)?;
    let (bv, bn) = split_null(b)?;
    if an || bn {
        let value = match (av, bv) {
            (Some(a), Some(b)) => merge(&a, &b, budget)?,
            (Some(v), None) | (None, Some(v)) => v,
            (None, None) => return Some(json!({"type":"null"})),
        };
        let mut items = vec![value, json!({"type":"null"})];
        items.sort_by_key(Value::to_string);
        return Some(json!({"anyOf":items}));
    }
    // Never discard extraction gaps, foreign constraints, or required/enum metadata.
    let (a, b) = (a.as_object()?, b.as_object()?);
    if a.len() != 2 || b.len() != 2 || a.get("type") != b.get("type") {
        return None;
    }
    match a.get("type")?.as_str()? {
        "object" => {
            let (ap, bp) = (
                a.get("properties")?.as_object()?,
                b.get("properties")?.as_object()?,
            );
            if !ap.keys().eq(bp.keys()) {
                return None;
            }
            let properties = ap
                .iter()
                .map(|(key, value)| Some((key.clone(), merge(value, &bp[key], budget)?)))
                .collect::<Option<serde_json::Map<_, _>>>()?;
            Some(json!({"type":"object","properties":properties}))
        }
        "array" => {
            let items = [a.get("items")?, b.get("items")?]
                .into_iter()
                .flat_map(|v| {
                    v.get("anyOf")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_else(|| vec![v.clone()])
                })
                .collect();
            Some(json!({"type":"array","items":alternatives(items)}))
        }
        _ => None,
    }
}

fn alternatives(items: Vec<Value>) -> Value {
    let unique: BTreeMap<_, _> = items.into_iter().map(|v| (v.to_string(), v)).collect();
    let mut out: Vec<Value> = vec![];
    let mut budget = OBSERVED_NODE_LIMIT;
    let mut values = unique.into_values();
    while let Some(value) = values.next() {
        if budget == 0 {
            out.push(value);
            out.extend(values);
            break;
        }
        if let Some((index, combined)) = out
            .iter()
            .enumerate()
            .find_map(|(i, v)| merge(v, &value, &mut budget).map(|v| (i, v)))
        {
            out[index] = combined;
        } else {
            out.push(value);
        }
    }
    out.sort_by_key(Value::to_string);
    match out.len() {
        0 => json!({"unknown":true}),
        1 => out.remove(0),
        _ => json!({"anyOf":out}),
    }
}

/// Project old stored observation shapes into the same comparison representation.
/// Does not rewrite revisions, source bytes, or field identities.
pub fn compact_observed_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let map: serde_json::Map<_, _> = map
                .iter()
                .map(|(k, v)| (k.clone(), compact_observed_schema(v)))
                .collect();
            if map.len() == 1
                && let Some(Value::Array(items)) = map.get("anyOf")
            {
                return alternatives(items.clone());
            }
            Value::Object(map)
        }
        Value::Array(items) => Value::Array(items.iter().map(compact_observed_schema).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_fields_nullable_combinations_have_one_object() {
        let values = json!([{"id":1,"name":null,"tasks":[]}, {"id":2,"name":"two","tasks":[{"id":3}]}, {"id":3,"name":null,"tasks":[{"id":4}]}]);
        let schema = observe_json(&values).schema;
        assert_eq!(schema["items"]["type"], "object");
        assert_eq!(schema["items"]["properties"].as_object().unwrap().len(), 3);
        assert_eq!(
            schema["items"]["properties"]["name"]["anyOf"],
            json!([{"type":"null"},{"type":"string"}])
        );
        assert_eq!(
            schema["items"]["properties"]["tasks"]["items"]["properties"]["id"]["type"],
            "number"
        );
        let mut reverse = values.as_array().unwrap().clone();
        reverse.reverse();
        assert_eq!(schema, observe_json(&json!(reverse)).schema);
        assert_eq!(schema, compact_observed_schema(&schema));
        assert!(!schema.to_string().contains("two"));
    }
    #[test]
    fn actual_type_conflicts_missing_keys_and_extraction_gaps_are_preserved() {
        for values in [
            json!([{"id":1},{"id":"1"}]),
            json!([{"id":1},{"id":2,"extra":true}]),
        ] {
            assert_eq!(
                observe_json(&values).schema["items"]["anyOf"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        }
        let legacy = json!({"anyOf":[{"type":"object","properties":{"id":{"type":"number"}}},{"type":"object","properties":{"id":{"type":"number"}},"x-observed-incomplete":true}]});
        assert_eq!(
            compact_observed_schema(&legacy)["anyOf"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(!crate::observed_schema_covers(&legacy, &legacy));
    }
    #[test]
    fn old_revision_comparison_is_compatible_without_rewriting_source() {
        let old = json!({"type":"array","items":{"anyOf":[{"type":"object","properties":{"id":{"type":"number"},"name":{"type":"null"}}},{"type":"object","properties":{"id":{"type":"number"},"name":{"type":"string"}}}]}});
        let original = old.clone();
        let new = observe_json(&json!([{"id":7,"name":"seven"},{"id":8,"name":null}])).schema;
        assert_eq!(compact_observed_schema(&old), new);
        assert!(crate::observed_schema_covers(&old, &new));
        assert!(crate::observed_schema_covers(&new, &old));
        assert_eq!(old, original);
        let changed = observe_json(&json!([{"id":false,"name":"seven"}])).schema;
        assert!(!crate::observed_schema_covers(&old, &changed));
        assert!(!crate::compare_observed_schemas(&old, &changed, "response.body").is_empty());
    }
    #[test]
    fn schema_keyword_named_business_fields_are_not_removed() {
        let schema = observe_json(&json!([{"anyOf":"business value"}])).schema;
        assert_eq!(compact_observed_schema(&schema), schema);
        assert_eq!(schema["items"]["properties"]["anyOf"]["type"], "string");
    }
}
