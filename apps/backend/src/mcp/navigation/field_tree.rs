//! Immediate field children; leaf rows do not advertise an empty expansion.
use super::{
    interface::{field_links, note_for},
    projection::{action, page, preview},
    *,
};
use nexofolio_contracts::{
    endpoint::{FieldFacts, FieldPath},
    knowledge::FieldKnowledge,
};

fn parent(f: &FieldFacts) -> Option<String> {
    if f.path.0.is_empty() {
        return None;
    }
    let mut p = f.clone();
    p.path = FieldPath(f.path.0[..f.path.0.len() - 1].to_vec());
    Some(nexofolio_contracts::knowledge::field_note_key(&p))
}

pub(super) fn read(
    q: &Query,
    fields: &[&FieldFacts],
    knowledge: &FieldKnowledge,
) -> Result<Value, ReadError> {
    let keys: std::collections::HashSet<_> = fields
        .iter()
        .map(|f| nexofolio_contracts::knowledge::field_note_key(f))
        .collect();
    if q.parent_key.as_ref().is_some_and(|key| !keys.contains(key)) {
        return Err("FIELD_NOT_FOUND".into());
    }
    let parents: Vec<_> = fields.iter().map(|f| parent(f)).collect();
    let mut child_counts = std::collections::HashMap::new();
    for parent in parents.iter().flatten() {
        *child_counts.entry(parent.as_str()).or_insert(0usize) += 1;
    }
    let items = fields
        .iter()
        .zip(&parents)
        .filter(|(f, p)| {
            (q.include_retired
                || q.endpoint_id
                    .is_some_and(|id| q.lifecycle.field(id, f.location, &f.path).is_none())
                || q.parent_key.is_some())
                && q.location.is_none_or(|l| f.location == l)
                && match &q.parent_key {
                    Some(wanted) => p.as_ref() == Some(wanted),
                    None => p.as_ref().is_none_or(|p| !keys.contains(p)),
                }
        })
        .map(|(f, _)| {
            let key = nexofolio_contracts::knowledge::field_note_key(f);
            let note = note_for(f, &knowledge.fields);
            let children = child_counts.get(key.as_str()).copied().unwrap_or(0);
            // key already carries the exact path and location. Full structured
            // path remains available in field_summary / field.
            let mut row = json!({
                "key":key,"location":f.location,"types":f.types,
                "name":note.and_then(|n|n.context.display_name.as_ref()),
                "description_preview":note.map(|n|preview(&n.text)),
                "children":children,"related_interfaces":field_links(q,&knowledge.links,f).len(),
                "open":action("read_interface",q,json!({"section":"field_summary","field_key":key}))
            });
            if children > 0 {
                row["expand"] = action(
                    "read_interface",
                    q,
                    json!({"section":"fields","parent_key":key}),
                );
            }
            row
        })
        .collect();
    page("read_interface", q, items)
}
