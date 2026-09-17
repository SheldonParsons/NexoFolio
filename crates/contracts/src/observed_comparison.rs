//! One bounded visitor: fast proof avoids allocating findings; detailed callers retain them.
use crate::{AssessmentKind as Kind, StructuralFinding};
use serde_json::Value;
const MAX_FINDINGS: usize = 512;
pub fn observed_schema_covers(a: &Value, b: &Value) -> bool {
    Comparison::new("", false).walk(
        &crate::compact_observed_schema(a),
        &crate::compact_observed_schema(b),
        "",
        false,
    )
}
pub fn compare_observed_schemas(a: &Value, b: &Value, location: &str) -> Vec<StructuralFinding> {
    let mut comparison = Comparison::new(location, true);
    comparison.walk(
        &crate::compact_observed_schema(a),
        &crate::compact_observed_schema(b),
        "",
        false,
    );
    comparison.findings.unwrap()
}
fn partial(v: &Value) -> bool {
    v["x-observed-incomplete"] == true || v["reason"] == "extraction_limit"
}
fn incomplete(v: &Value) -> bool {
    partial(v)
        || match v {
            Value::Object(m) => m.values().any(incomplete),
            Value::Array(a) => a.iter().any(incomplete),
            _ => false,
        }
}
fn child(path: &str, key: &str) -> String {
    format!("{}/{}", path, key.replace('~', "~0").replace('/', "~1"))
}
struct Comparison<'a> {
    location: &'a str,
    findings: Option<Vec<StructuralFinding>>,
    remaining: usize,
}
impl<'a> Comparison<'a> {
    fn new(location: &'a str, collect: bool) -> Self {
        Self {
            location,
            findings: collect.then(Vec::new),
            remaining: 100_000,
        }
    }
    fn change(
        &mut self,
        kind: Kind,
        reason: &str,
        path: &str,
        a: Option<&Value>,
        b: Option<&Value>,
    ) -> bool {
        if let Some(out) = &mut self.findings {
            if out.len() < MAX_FINDINGS {
                out.push(StructuralFinding {
                    kind,
                    location: self.location.into(),
                    path: path.into(),
                    reason: reason.into(),
                    before: a.cloned(),
                    observed: b.cloned(),
                });
            } else if out.len() == MAX_FINDINGS {
                out.push(StructuralFinding {
                    kind: Kind::Insufficient,
                    location: self.location.into(),
                    path: String::new(),
                    reason: "FINDING_LIMIT".into(),
                    before: None,
                    observed: None,
                });
            }
        }
        false
    }
    fn gap(&mut self, reason: &str, path: &str) -> bool {
        self.change(Kind::Insufficient, reason, path, None, None)
    }
    fn stop(&self, covered: bool) -> bool {
        !covered && self.findings.is_none()
    }
    fn walk(&mut self, a: &Value, b: &Value, path: &str, item: bool) -> bool {
        if self.remaining == 0 {
            return self.gap("COMPARISON_LIMIT", path);
        }
        self.remaining -= 1;
        let mut covered = true;
        if partial(a) || partial(b) {
            covered = self.gap("EXTRACTION_LIMIT", path);
            if self.stop(covered) || a["unknown"] == true || b["unknown"] == true {
                return false;
            }
        }
        if a == b {
            if !covered {
                return false;
            }
            return if incomplete(a) {
                self.gap("EXTRACTION_LIMIT", path)
            } else {
                covered
            };
        }
        if item
            && b.as_object()
                .is_some_and(|o| o.len() == 1 && o.get("unknown") == Some(&Value::Bool(true)))
        {
            return covered;
        }
        if a["unknown"] == true {
            let (kind, reason) = if item {
                (Kind::Enrichment, "ARRAY_ITEMS_OBSERVED")
            } else {
                (Kind::Insufficient, "BASELINE_UNKNOWN")
            };
            return self.change(kind, reason, path, Some(a), Some(b));
        }
        if b["unknown"] == true {
            return self.change(
                Kind::Insufficient,
                "OBSERVATION_UNKNOWN",
                path,
                Some(a),
                Some(b),
            );
        }
        if let Some(variants) = b["anyOf"].as_array() {
            if variants.is_empty() {
                return self.gap("EMPTY_VARIANTS", path);
            }
            for variant in variants {
                covered &= self.walk(a, variant, path, item);
                if self.stop(covered) {
                    return false;
                }
            }
            return covered;
        }
        if let Some(variants) = a["anyOf"].as_array() {
            let mut explanation: Option<Vec<StructuralFinding>> = None;
            let mut ambiguous = false;
            let mut incomplete = false;
            for variant in variants {
                let mut attempt = Self::new(self.location, self.findings.is_some());
                attempt.remaining = self.remaining;
                let found = attempt.walk(variant, b, path, item);
                self.remaining = attempt.remaining;
                if found {
                    return covered;
                }
                if let Some(findings) = attempt.findings {
                    incomplete |= findings.iter().any(|f| f.kind == Kind::Insufficient);
                    // Refinement is justified only by a compatible prior alternative.
                    // Do not select one by a magic similarity score.
                    if findings.iter().all(|f| f.kind != Kind::Difference) {
                        if let Some(previous) = &explanation {
                            ambiguous |= previous != &findings;
                        } else {
                            explanation = Some(findings);
                        }
                    }
                }
                if self.remaining == 0 {
                    return self.gap("COMPARISON_LIMIT", path);
                }
            }
            if ambiguous || explanation.is_none() {
                if incomplete || ambiguous {
                    self.gap("VARIANT_BASELINE_AMBIGUOUS", path);
                }
                return self.change(
                    Kind::Difference,
                    "OBSERVED_VARIANT_DIFFERS",
                    path,
                    Some(a),
                    Some(b),
                );
            }
            if let Some(explanation) = explanation {
                for f in explanation {
                    self.change(
                        f.kind,
                        &f.reason,
                        &f.path,
                        f.before.as_ref(),
                        f.observed.as_ref(),
                    );
                }
            }
            return false;
        }
        if a["type"] == "null" && b["type"] != "null" {
            return self.change(
                Kind::Enrichment,
                "NULL_TYPE_OBSERVED",
                path,
                Some(a),
                Some(b),
            );
        }
        if b["type"] == "null" {
            return self.change(
                Kind::Insufficient,
                "NULL_VALUE_NOT_TYPE_CHANGE",
                path,
                Some(a),
                Some(b),
            );
        }
        if a["type"] != b["type"] {
            return self.change(
                Kind::Difference,
                "OBSERVED_TYPE_DIFFERS",
                path,
                Some(a),
                Some(b),
            );
        }
        let structural = [
            "type",
            "properties",
            "items",
            "anyOf",
            "x-observed-incomplete",
        ];
        if let (Some(ka), Some(kb)) = (a.as_object(), b.as_object())
            && (ka
                .iter()
                .any(|(k, v)| !structural.contains(&k.as_str()) && kb.get(k) != Some(v))
                || kb
                    .keys()
                    .any(|k| !structural.contains(&k.as_str()) && !ka.contains_key(k)))
        {
            covered = self.gap("UNSUPPORTED_SCHEMA_METADATA_DIFFERS", path);
            if self.stop(covered) {
                return false;
            }
        }
        match b["type"].as_str() {
            Some("object") => {
                let (Some(ka), Some(kb)) =
                    (a["properties"].as_object(), b["properties"].as_object())
                else {
                    return self.gap("INVALID_OBSERVED_SCHEMA", path);
                };
                for (key, value) in ka {
                    let path = child(path, key);
                    covered &= if let Some(other) = kb.get(key) {
                        self.walk(value, other, &path, false)
                    } else {
                        self.change(
                            Kind::Insufficient,
                            "FIELD_NOT_OBSERVED",
                            &path,
                            Some(value),
                            None,
                        )
                    };
                    if self.stop(covered) {
                        return false;
                    }
                }
                for (key, value) in kb {
                    if !ka.contains_key(key) {
                        let (kind, reason) = if partial(a) {
                            (Kind::Enrichment, "BASELINE_FIELD_UNREAD")
                        } else {
                            (Kind::Difference, "FIELD_FIRST_OBSERVED")
                        };
                        covered &= self.change(kind, reason, &child(path, key), None, Some(value));
                        if self.stop(covered) {
                            return false;
                        }
                    }
                }
                covered
            }
            Some("array") => {
                self.walk(&a["items"], &b["items"], &format!("{path}/*"), true) && covered
            }
            _ => self.change(
                Kind::Difference,
                "OBSERVED_SHAPE_DIFFERS",
                path,
                Some(a),
                Some(b),
            ),
        }
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
    #[test]
    fn nullable_combinations_do_not_create_new_object_structures() {
        let a = crate::observe_json(&json!([{"a":null,"b":"x"},{"a":"x","b":null}])).schema;
        let b = crate::observe_json(&json!([{"a":"x","b":"x"}])).schema;
        let findings = compare_observed_schemas(&a, &b, "response.body");
        assert!(findings.is_empty());
        assert!(observed_schema_covers(&a, &b));
    }
    #[test]
    fn equal_nested_unread_shapes_are_not_proof_of_coverage() {
        let mut value = json!(1);
        for _ in 0..70 {
            value = json!({"child":value});
        }
        let shape = crate::observe_json(&value);
        assert!(
            shape
                .limitations
                .contains(&"STRUCTURE_EXTRACTION_LIMIT".into())
        );
        assert!(!observed_schema_covers(&shape.schema, &shape.schema));
    }
    #[test]
    fn additional_schema_metadata_cannot_be_ignored() {
        let a = json!({"type":"array","items":{"type":"number"},"minItems":1});
        let b = json!({"type":"array","items":{"type":"number"}});
        assert!(!observed_schema_covers(&a, &b));
        assert!(
            compare_observed_schemas(&a, &b, "response.body")
                .iter()
                .any(|f| f.kind == Kind::Insufficient)
        );
    }
}
