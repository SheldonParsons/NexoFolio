use nexofolio_contracts::{endpoint::EndpointFacts, knowledge::EndpointNote};

/// Call counts alone do not invalidate words. A new distinct structure does.
pub(crate) fn endpoint_needs_update(facts: &EndpointFacts, note: Option<&EndpointNote>) -> bool {
    let Some(note) = note else {
        return true;
    };
    if note.author.is_person() {
        return false;
    }
    let missing = |value: &Option<String>| value.as_ref().is_none_or(|s| s.trim().is_empty());
    missing(&note.name)
        || missing(&note.purpose)
        || facts
            .examples
            .iter()
            .any(|example| example.first_seen > note.at)
}
