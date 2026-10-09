//! Bound model work by actual input size without dropping evidence or fields.
use crate::FieldInput;
use nexofolio_contracts::knowledge::{EndpointKnowledge, EvidenceKind};
use serde_json::{Value, json};

pub const FIELD_BATCH_LIMIT: usize = 12;
pub const FIELD_BATCH_BYTES: usize = 16_000;

pub(crate) fn model_field(
    input: &FieldInput,
    index: usize,
    knowledge: &EndpointKnowledge,
) -> Value {
    json!({
        "key": index.to_string(), "path": input.path, "types": input.types,
        "values": input.values, "observations":input.observations,"representative_values":input.value_examples, "location": input.location, "parent": input.parent,
        "siblings": input.siblings,
        "clues": input.evidence.iter().filter(|e| e.kind != EvidenceKind::Inference)
            .map(|e| (&e.kind, &e.text, &e.options, &e.source)).collect::<Vec<_>>(),
        "previous": knowledge.fields.iter().find(|note| note.path == input.key).map(|note| json!({"display_name":note.context.display_name,"description":note.text,"completion":note.context.completion}))
    })
}

pub fn field_batches(
    inputs: Vec<FieldInput>,
    knowledge: &EndpointKnowledge,
) -> Vec<Vec<FieldInput>> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    for input in inputs {
        let mut trial = batch.clone();
        trial.push(input.clone());
        if !batch.is_empty()
            && (batch.len() == FIELD_BATCH_LIMIT
                || batch_bytes(&trial, knowledge) > FIELD_BATCH_BYTES)
        {
            batches.push(std::mem::take(&mut batch));
        }
        batch.push(input);
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

fn batch_bytes(inputs: &[FieldInput], knowledge: &EndpointKnowledge) -> usize {
    let fields = inputs
        .iter()
        .enumerate()
        .map(|(i, input)| {
            let mut field = model_field(input, i, knowledge);
            // Budget the longest completion request; actual partial retries may ask for less.
            field["requested"] = json!(["display_name", "description"]);
            field["preserve_completed"] = json!(false);
            field
        })
        .collect();
    super::shared_fields(fields).to_string().len()
}
