//! Full semantic meaning, bounded navigation references; evidence remains a separate read.
use super::*;
use super::{
    interface::{field_links, note_for},
    projection::{action, document},
};
use nexofolio_contracts::{
    endpoint::FieldFacts,
    knowledge::{EndpointKnowledge, EvidenceKind},
};
use std::collections::{BTreeMap, HashSet};
const REFERENCES: usize = 3;

pub(super) fn read(
    q: &Query,
    field: &FieldFacts,
    k: &EndpointKnowledge,
    summaries: &[EndpointSummary],
) -> Result<Value, ReadError> {
    let key = nexofolio_contracts::knowledge::field_note_key(field);
    let note = note_for(field, k);
    let mut links = field_links(q, k, field);
    links.sort_by_key(|l| l.id.to_string());
    let id = q.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
    let allowed: HashSet<_> = summaries.iter().map(|s| s.id).collect();
    let relations:Vec<_>=links.iter().take(REFERENCES).map(|l| {
  let other=if l.from==id{l.to}else{l.from};
  let target=summaries.iter().find(|s|s.id==other);
  json!({
   "link_id":l.id,"relation":l.relation,"confirmation":l.context.confirmation,
   "from":l.from,"to":l.to,"from_field":l.from_field,"to_field":l.to_field,
   "from_location":l.context.from_location,"to_location":l.context.to_location,"environment_id":l.context.environment,
   "related_interface":target.map(|s|json!({"endpoint_id":s.id,"method":s.method,"path":s.path_template,"open":action("read_interface",q,json!({"endpoint_id":s.id,"section":"overview"}))})),
   "evidence":action("read_source_evidence",q,json!({"link_id":l.id}))
  })
 }).collect();
    // A related-interface evidence reference is not promoted to a confirmed link.
    let mut kinds = BTreeMap::new();
    let mut related = Vec::new();
    let mut additional_total = 0;
    if let Some(n) = note {
        let all_linked: HashSet<_> = links
            .iter()
            .map(|l| if l.from == id { l.to } else { l.from })
            .collect();
        let mut seen = all_linked;
        for e in &n.context.evidence {
            let env = e
                .environment
                .or_else(|| e.source.as_ref().and_then(|s| s.environment));
            if q.environment_id
                .is_some_and(|wanted| env.is_some_and(|v| v != wanted))
            {
                continue;
            }
            let kind = serde_json::to_value(&e.kind).map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
            *kinds
                .entry(kind.as_str().unwrap_or("unknown").to_owned())
                .or_insert(0usize) += 1;
            if matches!(e.kind, EvidenceKind::RelatedInterface)
                && e.endpoint != id
                && allowed.contains(&e.endpoint)
                && seen.insert(e.endpoint)
            {
                additional_total += 1;
                if related.len() < REFERENCES {
                    related.push(json!({"endpoint_id":e.endpoint,"field":e.field,"kind":e.kind,"confirmation":"not_recorded","open":action("read_interface",q,json!({"endpoint_id":e.endpoint,"section":"overview"}))}));
                }
            }
        }
    }
    let evidence_count: usize = kinds.values().sum();
    let mut value = json!({
     "key":key,"path":field.path,"location":field.location,"types":field.types,"declared":field.declared,"conflict":field.conflict,
     "meaning":note.map(|n|json!({"text":n.text,"display_name":n.context.display_name,"display_name_confirmation":n.context.display_name_confirmation,"business_meaning":n.context.meaning,"generation_addressed":n.generation_addressed(),"completion":n.context.completion,"identity_scope":if n.context.location.is_some(){"qualified"}else{"legacy_location_unknown"}})),
     "relations":{"total":links.len(),"items":relations,"has_more":links.len()>REFERENCES,"open":action("read_interface",q,json!({"section":"relations","field_key":key}))},
     "related_evidence":{"total":additional_total,"items":related,"has_more":additional_total>REFERENCES,"open":note.map(|_|action("read_source_evidence",q,json!({"field_key":key})))},
     "evidence":{"total":evidence_count,"by_kind":kinds,"open":note.map(|_|action("read_source_evidence",q,json!({"field_key":key})))},
     "full":action("read_interface",q,json!({"section":"field","field_key":key})),
     "coverage":action("inspect_knowledge_coverage",q,json!({"section":"summary","field_key":key})),
     "samples":action("read_interface",q,json!({"section":"samples"})),
     "text_search":{"tool":"search_interfaces","scope":action("search_interfaces",q,json!({}))["arguments"],"required_input":"query","instruction":"Choose query terms from the complete field meaning yourself. An empty query enumerates all interfaces; it is not a targeted lookup."},
     "scope_note":"Semantic text is complete; evidence and values are deferred, not deleted. Meanings/types are saved project knowledge; check selected-environment evidence. Related evidence without a recorded relation retains unknown confirmation."
    });
    if let Some(n) = note
        && !n.values.is_empty()
    {
        value["legacy_value_preview"] = json!({"count":n.values.len(),"complete":false,"open":action("read_interface",q,json!({"section":"field","field_key":key}))});
    }
    if let Some(o) = note.and_then(|n| n.context.observations.as_ref()) {
        value["observations"] = json!({"recorded_total":o.total,"states":o.states,"scope":"saved_project_summary","open":o.archive_id.map(|archive|action("read_field_values",q,json!({"archive_id":archive,"field_key":key})))});
    }
    document("read_interface", q, value)
}
