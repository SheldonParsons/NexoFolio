//! Complete typed value extraction over retained observations. No model, storage or truncation.
mod parts;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldLocation, PathSegment, ValueType},
    field_values::*,
    knowledge::{EvidenceSource, field_note_key},
    observation::{CanonicalObservation, Fact},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

pub fn value_field_key(endpoint: nexofolio_common::EndpointId, field: &str) -> String {
    format!("{endpoint}/{field}")
}
pub fn value_summary(field: &ObservedField) -> FieldObservationSummary {
    let mut states = BTreeMap::new();
    for item in &field.coverage {
        *states.entry(item.state.clone()).or_insert(0) += 1;
    }
    FieldObservationSummary {
        archive_id: None,
        total: field.values.len(),
        hash: field.hash.clone(),
        states,
    }
}
pub fn archive_values(
    facts: &EndpointFacts,
    samples: &[(String, Option<Value>)],
) -> FieldValueArchive {
    let mut archive = FieldValueArchive {
        version: 1,
        ..Default::default()
    };
    let mut indices: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for field in &facts.fields {
        archive.fields.insert(
            value_field_key(facts.summary.id, &field_note_key(field)),
            ObservedField::default(),
        );
    }
    for (example, raw) in samples {
        let observation = raw
            .as_ref()
            .and_then(|raw| serde_json::from_value::<CanonicalObservation>(raw.clone()).ok());
        let mut locations = BTreeMap::new();
        if let Some(observation) = &observation
            && let Fact::Exchange(exchange) = &observation.fact
        {
            for field in &facts.fields {
                locations.entry(field.location).or_insert_with(|| {
                    parts::part(exchange, field.location, &facts.summary.path_template)
                });
            }
        }
        for field in &facts.fields {
            let key = value_field_key(facts.summary.id, &field_note_key(field));
            let source = EvidenceSource {
                endpoint: Some(facts.summary.id),
                example: example.clone(),
                environment: observation.as_ref().and_then(|o| o.environment_id),
                location: Some(field.location),
                page: observation
                    .as_ref()
                    .and_then(|o| o.context.as_ref())
                    .and_then(|c| c.get("page_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                sequence: observation
                    .as_ref()
                    .and_then(|o| o.context.as_ref())
                    .and_then(|c| c.get("seq"))
                    .and_then(Value::as_u64),
                platform: observation.as_ref().map(|o| o.source.platform.clone()),
                ..Default::default()
            };
            let Some(part) = locations.get(&field.location).and_then(Option::as_ref) else {
                // A different response status is not evidence of absence.
                if observation.as_ref().is_none_or(|o| {
                    matches!(&o.fact,Fact::Exchange(e) if e.response.is_none())
                        || !matches!(&o.fact, Fact::Exchange(_))
                }) {
                    archive
                        .fields
                        .get_mut(&key)
                        .unwrap()
                        .coverage
                        .push(ValueCoverage {
                            source,
                            state: "unavailable".into(),
                        });
                }
                continue;
            };
            let mut source = source;
            source.complete = Some(part.state == "full");
            let Some(root) = &part.root else {
                archive
                    .fields
                    .get_mut(&key)
                    .unwrap()
                    .coverage
                    .push(ValueCoverage {
                        source,
                        state: part.state.clone(),
                    });
                continue;
            };
            let original = field.path.0.is_empty() && part.original.is_some();
            let root = if original {
                part.original.as_ref().unwrap()
            } else {
                root
            };
            let representation = if original {
                "urlencoded_raw"
            } else {
                &part.representation
            };
            let document = hash(&serde_json::json!([
                facts.summary.id,
                example,
                field.location,
                representation
            ]));
            if !archive.documents.contains_key(&document) {
                archive.documents.insert(
                    document.clone(),
                    ValueDocument {
                        source: source.clone(),
                        observed_at: observation.as_ref().map(|o| o.observed_at),
                        observation_key: observation.as_ref().map(|o| o.key),
                        representation: representation.into(),
                        state: part.state.clone(),
                        root: root.clone(),
                    },
                );
            }
            let mut found = Vec::new();
            if part.representation == "multipart_summary" && !field.path.0.is_empty() {
                if let Some(PathSegment::Key(name)) = field.path.0.first()
                    && let Some(entries) = root.as_array()
                {
                    for (index, entry) in entries.iter().enumerate() {
                        if entry.get("name").and_then(Value::as_str) == Some(name) {
                            if let Some(value) = entry.get("value") {
                                at(
                                    value,
                                    &field.path.0[1..],
                                    &format!("/{index}/value"),
                                    &mut found,
                                );
                            } else if entry.get("file").is_some() {
                                at(entry, &field.path.0[1..], &format!("/{index}"), &mut found);
                            }
                        }
                    }
                }
            } else if part.pairs && !field.path.0.is_empty() {
                if let Some(PathSegment::Key(name)) = field.path.0.first()
                    && let Some(pairs) = root.as_array()
                {
                    for (index, pair) in pairs.iter().enumerate() {
                        let Some(key) = pair.get(0).and_then(Value::as_str) else {
                            continue;
                        };
                        let matches = if matches!(
                            field.location,
                            FieldLocation::RequestHeader | FieldLocation::ResponseHeader { .. }
                        ) {
                            key.eq_ignore_ascii_case(name)
                        } else {
                            key == name
                        };
                        if matches && let Some(value) = pair.get(1) {
                            at(
                                value,
                                &field.path.0[1..],
                                &format!("/{index}/1"),
                                &mut found,
                            );
                        }
                    }
                }
            } else {
                at(root, &field.path.0, "", &mut found);
            }
            let target = archive.fields.get_mut(&key).unwrap();
            target.coverage.push(ValueCoverage {
                source,
                state: if found.is_empty()
                    && !part.pairs
                    && matches!(
                        part.representation.as_str(),
                        "text" | "binary" | "multipart_raw"
                    ) {
                    "unreadable_structure".into()
                } else if found.is_empty() && part.state == "full" {
                    "absent".into()
                } else {
                    part.state.clone()
                },
            });
            let index = indices.entry(key).or_default();
            for (pointer, value) in found {
                let id = hash(&serde_json::json!([representation, value]));
                let next = target.values.len();
                let position = *index.entry(id.clone()).or_insert(next);
                if position == next {
                    target.values.push(ObservedValueRef {
                        id,
                        value_type: kind(value),
                        representation: representation.into(),
                        occurrences: vec![],
                    });
                }
                target.values[position].occurrences.push(ValuePointer {
                    document: document.clone(),
                    pointer,
                });
            }
        }
    }
    for field in archive.fields.values_mut() {
        field.hash = hash(&serde_json::json!([field.values, field.coverage]));
    }
    archive
}
fn hash(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("values serialize"))
    )
}
fn kind(value: &Value) -> ValueType {
    match value {
        Value::Null => ValueType::Null,
        Value::Bool(_) => ValueType::Boolean,
        Value::Number(_) => ValueType::Number,
        Value::String(_) => ValueType::String,
        Value::Array(_) => ValueType::Array,
        Value::Object(_) => ValueType::Object,
    }
}
fn at<'a>(
    value: &'a Value,
    path: &[PathSegment],
    pointer: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match path.split_first() {
        None => out.push((pointer.into(), value)),
        Some((PathSegment::Key(key), rest)) => {
            if let Some(value) = value.get(key) {
                at(
                    value,
                    rest,
                    &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    out,
                );
            }
        }
        Some((PathSegment::Items, rest)) => {
            if let Some(items) = value.as_array() {
                for (index, value) in items.iter().enumerate() {
                    at(value, rest, &format!("{pointer}/{index}"), out);
                }
            }
        }
    }
}
