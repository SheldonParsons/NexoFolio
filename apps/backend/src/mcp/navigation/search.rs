//! Deterministic explicit search. No scenario names, synonyms or model-generated routing.
use super::*;
use super::{
    catalogue::{in_environment, selected},
    projection::{action, page, preview},
};
use nexofolio_contracts::knowledge::EndpointNote;
pub(super) fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}
fn identity(s: &EndpointSummary, n: Option<&EndpointNote>) -> Value {
    json!({"endpoint_id":s.id,"method":s.method,"path":s.path_template,"name":n.and_then(|n|n.name.as_ref()),
 "purpose":n.and_then(|n|n.purpose.as_deref()).map(preview),"purpose_preview":true})
}
pub(super) fn card(q: &Query, s: &EndpointSummary, n: Option<&EndpointNote>) -> Value {
    let mut row = identity(s, n);
    row["environments"] = json!(s.environments);
    row["open"] = action(
        "read_interface",
        q,
        json!({"endpoint_id":s.id,"section":"overview"}),
    );
    row
}
impl Navigation {
    pub(super) async fn search(
        &self,
        q: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        let mode = q.search_in.as_deref().unwrap_or("interface");
        if !matches!(mode, "interface" | "fields") {
            return Err("INVALID_ARGUMENTS".into());
        }
        let snapshot = self
            .knowledge
            .catalogue_navigation(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let members = Some(selected(q, &snapshot, summaries)?);
        let notes = self
            .knowledge
            .notes(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let field_notes = if mode == "fields" {
            self.knowledge
                .field_notes(q.project_id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
        } else {
            Vec::new()
        };
        let mut items = Vec::new();
        let mut sorted: Vec<_> = summaries.iter().collect();
        sorted.sort_by_key(|s| (q.lifecycle.endpoint(s.id).is_some(), s.id.to_string()));
        for s in sorted {
            if !super::lifecycle::visible(q, s.id, None)
                || !in_environment(s, q.environment_id)
                || members.as_ref().is_some_and(|m| !m.contains(&s.id))
                || q.method
                    .as_ref()
                    .is_some_and(|m| !s.method.eq_ignore_ascii_case(m))
                || q.business_scope.as_ref().is_some_and(|scope| {
                    !s.path_template
                        .to_lowercase()
                        .contains(&scope.to_lowercase())
                })
            {
                continue;
            }
            let n = notes.iter().find(|n| n.endpoint == s.id);
            if mode == "interface" {
                let text = format!(
                    "{} {} {} {}",
                    s.method,
                    s.path_template,
                    n.and_then(|n| n.name.as_deref()).unwrap_or(""),
                    n.and_then(|n| n.purpose.as_deref()).unwrap_or("")
                );
                if matches(&q.query, &text) {
                    items.push(card(q, s, n));
                }
            } else {
                for note in field_notes.iter().filter(|n| {
                    n.endpoint == s.id && super::lifecycle::visible(q, s.id, Some(&n.path))
                }) {
                    let key = &note.path;
                    let text = format!(
                        "{} {} {}",
                        key,
                        note.context.display_name.as_deref().unwrap_or(""),
                        note.text
                    );
                    if matches(&q.query, &text) {
                        // A precise field has its own complete read action.
                        // Traffic counters and a duplicate interface action do
                        // not help choose a field; overview remains readable by ID.
                        let mut row = identity(s, n);
                        let open = if note.context.location.is_some() {
                            action(
                                "read_interface",
                                q,
                                json!({"endpoint_id":s.id,"section":"field_summary","field_key":key}),
                            )
                        } else {
                            action(
                                "read_interface",
                                q,
                                json!({"endpoint_id":s.id,"section":"fields"}),
                            )
                        };
                        row["matched_field"] = json!({"key":key,"name":note.context.display_name,"location":note.context.location,"description":preview(&note.text),"definition_scope":"saved_project_knowledge","identity_state":if note.context.location.is_some(){"qualified"}else{"legacy_location_unknown"},"open":open});
                        items.push(row);
                    }
                }
            }
        }
        for item in &mut items {
            let placement = snapshot
                .placements
                .iter()
                .find(|p| json!(p.endpoint) == item["endpoint_id"]);
            item["directory_path"] = json!(
                placement
                    .map(|p| super::catalogue::breadcrumb(&snapshot.catalogue.folders, p.folder))
                    .unwrap_or_default()
            );
            item["directory_state"] = json!(if placement.is_some() {
                "placed"
            } else {
                "unclassified"
            });
        }
        page("search_interfaces", q, items)
    }
}
