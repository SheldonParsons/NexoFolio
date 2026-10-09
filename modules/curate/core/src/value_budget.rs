//! The model sees a representative budget; the immutable archive retains every typed value.
use nexofolio_contracts::field_values::{FieldValueArchive, ObservedField};
use serde_json::{Value, json};
pub fn model_value_examples(archive: &FieldValueArchive, field: &ObservedField) -> Vec<Value> {
    let count = field.values.len();
    let indices: std::collections::BTreeSet<_> = if count <= 12 {
        (0..count).collect()
    } else {
        (0..6).chain(count - 6..count).collect()
    };
    indices.into_iter().filter_map(|index| {
        let entry=&field.values[index];let pointer=entry.occurrences.first()?;let document=archive.documents.get(&pointer.document)?;let value=document.root.pointer(&pointer.pointer)?;
        let encoded=serde_json::to_string(value).ok()?;
        Some(if encoded.len()<=256 {json!({"value_json":encoded,"type":entry.value_type,"representation":entry.representation,"value_id":entry.id,"source":document.source})}
            else {json!({"type":entry.value_type,"representation":entry.representation,"value_id":entry.id,"json_bytes":encoded.len(),"source":document.source,"value_omitted_from_model":true})})
    }).collect()
}
