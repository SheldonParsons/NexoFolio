//! Stable inputs to directory planning. Processing timestamps are not business changes.
use nexofolio_contracts::{
    endpoint::EndpointSummary,
    knowledge::{EndpointNote, Folder, Placement},
};
use sha2::{Digest, Sha256};
pub fn directory_input_hash(
    summaries: &[EndpointSummary],
    notes: &[EndpointNote],
    folders: &[Folder],
    placements: &[Placement],
) -> String {
    let mut endpoints: Vec<_> = summaries
        .iter()
        .map(|s| {
            let note = notes.iter().find(|n| n.endpoint == s.id);
            (
                s.id,
                s.method.clone(),
                s.path_template.clone(),
                note.and_then(|n| n.name.clone()),
                note.and_then(|n| n.purpose.clone()),
                note.and_then(|n| n.context.description.clone()),
            )
        })
        .collect();
    endpoints.sort_by_key(|row| row.0.to_string());
    let mut tree: Vec<_> = folders
        .iter()
        .map(|f| (f.id, f.parent, &f.name, &f.summary, f.position, &f.author))
        .collect();
    tree.sort_by_key(|row| row.0.to_string());
    let mut seats: Vec<_> = placements
        .iter()
        .map(|p| (p.endpoint, p.folder, &p.author))
        .collect();
    seats.sort_by_key(|row| row.0.to_string());
    let input = serde_json::json!(["directory-knowledge-v1", endpoints, tree, seats]);
    format!("{:x}", Sha256::digest(input.to_string().as_bytes()))
}
