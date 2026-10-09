use crate::FieldInput;
use nexofolio_contracts::{
    endpoint::EndpointFacts,
    knowledge::{EndpointKnowledge, EndpointNote},
};
use sha2::{Digest, Sha256};

/// The recipe is unchanged by a structural refactor. Semantic changes must bump it.
const FIELD_RECIPE: &str = "field-knowledge-v5";
const LEGACY_RECIPE: &str = "field-knowledge-v3";

pub fn field_input_hash(
    facts: &EndpointFacts,
    note: Option<&EndpointNote>,
    input: &FieldInput,
) -> String {
    hash_with_recipe(facts, note, input, FIELD_RECIPE)
}

pub fn legacy_field_input_hash(
    facts: &EndpointFacts,
    note: Option<&EndpointNote>,
    input: &FieldInput,
) -> String {
    hash_with_recipe(facts, note, input, LEGACY_RECIPE)
}
fn hash_with_recipe(
    facts: &EndpointFacts,
    note: Option<&EndpointNote>,
    input: &FieldInput,
    recipe: &str,
) -> String {
    // Version the reasoning recipe too: improving curation invalidates old
    // interpretations even when the captured JSON structure stays unchanged.
    let mut stable_input = serde_json::to_value(input).expect("field input");
    if let Some(object) = stable_input.as_object_mut() {
        object.remove("value_examples");
    }
    if let Some(observations) = stable_input
        .get_mut("observations")
        .and_then(serde_json::Value::as_object_mut)
    {
        observations.remove("archive_id");
    }
    let source = serde_json::json!([
        recipe,
        facts.summary.method,
        facts.summary.path_template,
        note.map(|n| (&n.name, &n.purpose)),
        stable_input
    ]);
    format!("{:x}", Sha256::digest(source.to_string().as_bytes()))
}

pub fn pending_fields(
    facts: &EndpointFacts,
    knowledge: &EndpointKnowledge,
    inputs: Vec<FieldInput>,
) -> Vec<FieldInput> {
    inputs
        .into_iter()
        .filter(|input| {
            let previous = knowledge.fields.iter().find(|n| {
                n.path == input.key || (n.path == input.path && n.context.location.is_none())
            });
            previous.is_none_or(|note| {
                !note.author.is_person()
                    && (!field_addressed(note)
                        || (!fresh(facts, knowledge.note.as_ref(), input, note)))
            })
        })
        .collect()
}

pub(crate) fn fresh(
    facts: &EndpointFacts,
    endpoint: Option<&EndpointNote>,
    input: &FieldInput,
    previous: &nexofolio_contracts::knowledge::FieldNote,
) -> bool {
    let stored = previous.context.input_hash.as_deref();
    if stored == Some(&field_input_hash(facts, endpoint, input)) {
        return true;
    }
    // v4 changes the boundary between structural metadata and explanatory prose.
    // Clean v3 meaning remains reusable while its exact evidence is unchanged.
    let structural = previous.context.meaning.details.iter().any(|s| {
        ["位置：", "父级结构：", "兄弟字段：", "类型：", "来源：真实"]
            .iter()
            .any(|prefix| s.starts_with(prefix))
    });
    !structural && stored == Some(&legacy_field_input_hash(facts, endpoint, input))
}

/// Completed output is distinct from confirmation of its business meaning.
pub fn field_addressed(note: &nexofolio_contracts::knowledge::FieldNote) -> bool {
    note.context.completion.as_ref().map_or_else(
        || {
            !note.text.trim().is_empty()
                && note
                    .context
                    .display_name
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty())
        },
        |c| c.addressed(note.context.display_name.as_deref(), &note.text),
    )
}
