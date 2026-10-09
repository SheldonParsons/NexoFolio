//! Collected endpoint additions can extend a draft; official knowledge changes cannot.
use nexofolio_contracts::knowledge::KnowledgeSnapshot;

pub(super) fn compatible_base(previous: &KnowledgeSnapshot, current: &KnowledgeSnapshot) -> bool {
    previous
        .endpoints
        .iter()
        .all(|id| current.endpoints.contains(id))
        && previous.folders == current.folders
        && previous.placements == current.placements
        && previous.notes == current.notes
        && previous.fields == current.fields
        && previous.links == current.links
}
