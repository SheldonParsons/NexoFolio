//! Deterministic explicit search. No scenario names, synonyms or model-generated routing.
use super::*;
use super::{
    catalogue::{in_environment, selected},
    projection::{action, paged, preview},
};
use nexofolio_contracts::knowledge::EndpointNote;
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
        let members = selected(q, &snapshot, summaries)?;
        let mut sorted: Vec<_> = summaries
            .iter()
            .filter(|s| {
                super::lifecycle::visible(q, s.id, None)
                    && in_environment(s, q.environment_id)
                    && members.contains(&s.id)
                    && q.method
                        .as_ref()
                        .is_none_or(|m| s.method.eq_ignore_ascii_case(m))
                    && q.business_scope.as_ref().is_none_or(|scope| {
                        s.path_template
                            .to_lowercase()
                            .contains(&scope.to_lowercase())
                    })
            })
            .collect();
        sorted.sort_by_key(|s| (q.lifecycle.endpoint(s.id).is_some(), s.id.to_string()));
        let endpoints = sorted
            .iter()
            .map(|s| {
                let mut scope = nexofolio_contracts::knowledge::SearchEndpoint {
                    id: s.id,
                    method: s.method.clone(),
                    path: s.path_template.clone(),
                    excluded_keys: Vec::new(),
                    excluded_prefixes: Vec::new(),
                };
                if !q.include_retired {
                    for decision in &q.lifecycle.decisions {
                        if let nexofolio_contracts::knowledge::LifecycleTarget::Field {
                            location,
                            path,
                            ..
                        } = &decision.target
                        {
                            let key = nexofolio_contracts::knowledge::qualified_field_key(
                                *location, path,
                            );
                            if q.lifecycle.field_key(s.id, &key).is_some() {
                                scope.excluded_keys.push(key.clone());
                                if path.0.is_empty() {
                                    scope.excluded_prefixes.push(key);
                                } else {
                                    scope
                                        .excluded_prefixes
                                        .extend([format!("{key}."), format!("{key}[]")]);
                                }
                            }
                        }
                    }
                }
                scope
            })
            .collect();
        let query = nexofolio_contracts::knowledge::KnowledgeSearch {
            project: q.project_id,
            endpoints,
            fields: mode == "fields",
            words: q
                .query
                .to_lowercase()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            offset: q.offset,
            limit: q.limit,
        };
        let found = self
            .knowledge
            .search(query)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let mut items = Vec::new();
        for hit in found.items {
            let s = sorted
                .iter()
                .find(|s| s.id == hit.endpoint)
                .ok_or("KNOWLEDGE_UNAVAILABLE")?;
            let mut row = json!({"endpoint_id":s.id,"method":s.method,"path":s.path_template,
                "name":hit.name,"purpose":hit.purpose,"purpose_preview":true});
            if let Some(field) = hit.field {
                let open = if field.location.is_some() {
                    action(
                        "read_interface",
                        q,
                        json!({"endpoint_id":s.id,"section":"field_summary","field_key":field.key}),
                    )
                } else {
                    action(
                        "read_interface",
                        q,
                        json!({"endpoint_id":s.id,"section":"fields"}),
                    )
                };
                row["matched_field"] = json!({"key":field.key,"name":field.name,"location":field.location,
                    "description":field.description,"definition_scope":"saved_project_knowledge",
                    "identity_state":if field.location.is_some(){"qualified"}else{"legacy_location_unknown"},"open":open});
            } else {
                row["environments"] = json!(s.environments);
                row["open"] = action(
                    "read_interface",
                    q,
                    json!({"endpoint_id":s.id,"section":"overview"}),
                );
            }
            items.push(row);
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
        // Scope/context carry identity and directory changes, never live counters.
        let context = json!({"source":found.revision,"directories":snapshot.catalogue.folders,
            "placements":snapshot.placements.iter().map(|p| (p.endpoint,p.folder)).collect::<Vec<_>>(),
            "endpoints":sorted.iter().map(|s| (s.id,&s.method,&s.path_template,
                s.environments.iter().map(|e|e.environment_id).collect::<Vec<_>>())).collect::<Vec<_>>()});
        paged("search_interfaces", q, items, found.total, &context)
    }
}
