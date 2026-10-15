//! Coverage dimensions are separate: generated output, semantic confirmation and capture.
use super::*;
use super::{
    catalogue::{in_environment, selected},
    interface::{field_visible, note_for},
    projection::{action, page},
};
use nexofolio_contracts::knowledge::Confirmation;
use std::collections::BTreeMap;

fn open(q: &Query, kind: &str) -> Value {
    action(
        "inspect_knowledge_coverage",
        q,
        json!({"directory_id":q.directory_id,"recursive":q.recursive,"field_key":q.field_key,"section":"issues","issue_kind":kind}),
    )
}
fn category(kind: &str) -> &'static str {
    match kind {
        "field_meaning_unconfirmed" => "semantic_confirmation",
        "field_generation_incomplete" => "generation",
        "unclassified" => "organization",
        "capture_gap" | "request_material_not_retained" => "capture",
        _ => "documentation",
    }
}
impl Navigation {
    pub(super) async fn coverage(
        &self,
        q: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        let modern = q.section.is_some() || q.issue_kind.is_some() || q.field_key.is_some();
        let section = q.section.as_deref().unwrap_or("issues");
        if !matches!(section, "summary" | "issues") {
            return Err("INVALID_ARGUMENTS".into());
        }
        if q.endpoint_id.is_some() && q.directory_id.is_some() {
            return Err("INVALID_ARGUMENTS".into());
        }
        if q.field_key.is_some() && q.endpoint_id.is_none() {
            return Err("ENDPOINT_REQUIRED".into());
        }
        if section == "summary" && (q.offset != 0 || q.revision.is_some() || q.issue_kind.is_some())
        {
            return Err("SECTION_ARGUMENT_MISMATCH".into());
        }
        let snapshot = self
            .knowledge
            .catalogue_navigation(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let ids = selected(q, &snapshot, summaries)?;
        let mut sorted: Vec<_> = summaries
            .iter()
            .filter(|s| {
                (q.endpoint_id.is_some() || q.lifecycle.endpoint(s.id).is_none())
                    && ids.contains(&s.id)
                    && in_environment(s, q.environment_id)
                    && q.endpoint_id.is_none_or(|id| s.id == id)
            })
            .collect();
        sorted.sort_by_key(|s| s.id.to_string());
        let mut issues = Vec::new();
        let mut fields = 0;
        let mut described = 0;
        let mut addressed = 0;
        let mut retained = 0;
        let mut semantic = BTreeMap::new();
        let mut names = BTreeMap::new();
        for s in &sorted {
            let f = self
                .endpoints
                .get(s.id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                .ok_or("NOT_FOUND")?;
            let k = self
                .knowledge
                .endpoint(s.id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
            let mut scope = q.clone();
            scope.endpoint_id = Some(s.id);
            if q.field_key.is_none() {
                if k.note.as_ref().is_none_or(|n| {
                    n.name.as_ref().is_none_or(|s| s.trim().is_empty())
                        || n.purpose.as_ref().is_none_or(|s| s.trim().is_empty())
                }) {
                    issues.push(json!({"kind":"interface_description_missing","endpoint_id":s.id,"open":action("read_interface",&scope,json!({"section":"overview"}))}));
                }
                if k.placement.is_none() {
                    issues.push(json!({"kind":"unclassified","endpoint_id":s.id,"open":action("read_interface",&scope,json!({"section":"overview"}))}));
                }
            }
            let visible: Vec<_> = f
                .fields
                .iter()
                .filter(|field| {
                    field_visible(field, q.environment_id)
                        && (q.field_key.is_some()
                            || q.lifecycle
                                .field(s.id, field.location, &field.path)
                                .is_none())
                })
                .filter(|field| {
                    q.field_key.as_ref().is_none_or(|key| {
                        nexofolio_contracts::knowledge::field_note_key(field) == *key
                    })
                })
                .collect();
            if q.field_key.is_some() && visible.is_empty() {
                return Err("FIELD_NOT_FOUND".into());
            }
            for field in visible {
                fields += 1;
                let note = note_for(field, &k.fields);
                let key = nexofolio_contracts::knowledge::field_note_key(field);
                let has_text = note.is_some_and(|n| !n.text.trim().is_empty());
                if has_text {
                    described += 1;
                }
                let complete = note.is_some_and(|n| n.generation_addressed());
                if complete {
                    addressed += 1;
                }
                let confirmation = note.map(|n| {
                    serde_json::to_value(n.context.meaning.confirmation).expect("confirmation")
                });
                *semantic
                    .entry(
                        confirmation
                            .as_ref()
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                    )
                    .or_insert(0usize) += 1;
                let name = note.map(|n| {
                    serde_json::to_value(n.context.display_name_confirmation).expect("confirmation")
                });
                *names
                    .entry(
                        name.as_ref()
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                    )
                    .or_insert(0usize) += 1;
                let mut kinds = Vec::new();
                if !has_text {
                    kinds.push("field_description_missing");
                }
                if modern && !complete {
                    kinds.push("field_generation_incomplete");
                }
                if has_text
                    && note.is_some_and(|n| {
                        matches!(n.context.meaning.confirmation, Confirmation::Candidate)
                    })
                {
                    kinds.push("field_meaning_unconfirmed");
                }
                for kind in kinds {
                    issues.push(json!({"kind":kind,"endpoint_id":s.id,"field_key":key,"open":action("read_interface",&scope,json!({"section":if modern{"field_summary"}else{"field"},"field_key":key}))}));
                }
            }
            // Field scope inspects only that field, not unrelated samples or interface gaps.
            if q.field_key.is_some() {
                continue;
            }
            let materials: Vec<_> = f
                .requests
                .iter()
                .filter(|r| q.environment_id.is_none_or(|e| r.environment_id == Some(e)))
                .collect();
            retained += materials.len();
            if materials.is_empty() {
                issues.push(json!({"kind":"request_material_not_retained","endpoint_id":s.id,"open":action("read_interface",&scope,json!({"section":"samples"}))}));
            }
            for r in materials {
                for gap in &r.gaps {
                    if gap.starts_with("current_business_authentication_unverified") {
                        continue;
                    }
                    issues.push(json!({"kind":"capture_gap","endpoint_id":s.id,"example_id":r.example_id,"gap":gap,"open":action("read_request_material",&scope,json!({"example_id":r.example_id}))}));
                }
            }
        }
        let stats = json!({"interfaces":sorted.len(),"fields":fields,"fields_with_descriptions":described,"generation_addressed":addressed,"generation_not_addressed":fields-addressed,"semantic_confirmation":semantic,"name_confirmation":names,"capture_inspected":q.field_key.is_none(),"retained_request_materials":if q.field_key.is_none(){Some(retained)}else{None}});
        let boundary = "Only saved facts/official knowledge are inspected. Generated completeness and semantic confirmation are different dimensions. Categories can overlap on one field; counts are not independent business failures. No known gaps does not establish completeness of business APIs; unknown business APIs are outside this coverage.";
        if section == "summary" {
            let mut counts = BTreeMap::new();
            for issue in &issues {
                let kind = issue["kind"].as_str().expect("issue kind");
                *counts.entry(kind).or_insert(0usize) += 1;
            }
            let groups:Vec<_>=counts.into_iter().map(|(kind,count)|json!({"kind":kind,"category":category(kind),"count":count,"open":open(q,kind)})).collect();
            return Ok(
                json!({"view":"summary","project_id":q.project_id,"environment_id":q.environment_id,"directory_id":q.directory_id,"endpoint_id":q.endpoint_id,"field_key":q.field_key,"knowledge_view":"official","coverage":stats,"issue_records":issues.len(),"groups":groups,"boundary":boundary}),
            );
        }
        if let Some(kind) = &q.issue_kind {
            issues.retain(|i| i["kind"].as_str() == Some(kind.as_str()));
        }
        let mut out = page("inspect_knowledge_coverage", q, issues)?;
        out["coverage"] = stats;
        out["boundary"] = json!(boundary);
        Ok(out)
    }
}
