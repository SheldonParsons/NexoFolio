//! Light interface entry, direct field expansion and explicit relationship navigation.
use super::projection::{action, document, page, preview};
use super::*;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldFacts},
    knowledge::{EndpointKnowledge, FieldNote, Link},
};
pub(super) fn field_visible(f: &FieldFacts, env: Option<EnvironmentId>) -> bool {
    env.is_none_or(|e| {
        f.labels.is_empty()
            || f.labels.iter().any(|l| {
                l.environment_id == e
                    && !matches!(l.label, nexofolio_contracts::endpoint::FieldLabel::Absent)
            })
    })
}
pub(super) fn note_for<'a>(f: &FieldFacts, k: &'a EndpointKnowledge) -> Option<&'a FieldNote> {
    let key = nexofolio_contracts::knowledge::field_note_key(f);
    k.fields
        .iter()
        .find(|n| n.path == key || (n.context.location.is_none() && n.path == f.path.to_string()))
}
pub(super) fn field_links<'a>(
    q: &Query,
    k: &'a EndpointKnowledge,
    f: &FieldFacts,
) -> Vec<&'a Link> {
    let endpoint = q.endpoint_id.expect("field endpoint");
    let path = f.path.to_string();
    k.links
        .iter()
        .filter(|l| {
            q.environment_id
                .is_none_or(|e| l.context.environment.is_none_or(|v| v == e))
        })
        .filter(|l| {
            (l.from == endpoint
                && l.from_field.as_ref() == Some(&path)
                && l.context.from_location.is_none_or(|v| v == f.location))
                || (l.to == endpoint
                    && l.to_field.as_ref() == Some(&path)
                    && l.context.to_location.is_none_or(|v| v == f.location))
        })
        .collect()
}
pub(super) fn link_card(q: &Query, l: &Link) -> Value {
    let endpoint = q.endpoint_id.expect("relation endpoint");
    let related = if l.from == endpoint { l.to } else { l.from };
    json!({"link_id":l.id,"lifecycle":{"state":if q.lifecycle.link_retired(l){"retired_source"}else{"active"}},"source_notice":if q.lifecycle.link_retired(l){Some("关联来源已废弃")}else{None},"from":l.from,"to":l.to,"from_field":l.from_field,"to_field":l.to_field,
 "from_location":l.context.from_location,"to_location":l.context.to_location,"relation":l.relation,
 "confirmation":l.context.confirmation,"environment_id":l.context.environment,"condition":l.context.condition,
 "evidence_preview":preview(&l.evidence),"evidence_preview_truncated":l.evidence.chars().count()>320,
 "related_interface":action("read_interface",q,json!({"endpoint_id":related,"section":"overview"})),
 "evidence":action("read_source_evidence",q,json!({"link_id":l.id}))})
}
fn sample_cards(q: &Query, f: &EndpointFacts) -> Vec<Value> {
    let mut rows = Vec::new();
    for s in &f.examples {
        if q.environment_id.is_some_and(|e| s.environment_id != e) {
            continue;
        }
        rows.push(json!({"example_id":s.id,"environment_id":s.environment_id,"status":s.status,"first_seen":s.first_seen,"last_seen":s.last_seen,
  "same_structure_calls":s.calls,"retained_samples":s.retained_samples,
  "request":action("read_request_material",q,json!({"example_id":s.id})),"response":action("read_response_material",q,json!({"example_id":s.id})),"context":action("read_source_evidence",q,json!({"example_id":s.id}))}));
    }
    // Some legacy facts retain request material but do not have ExampleSummary rows.
    for r in &f.requests {
        if q.environment_id
            .is_some_and(|e| r.environment_id != Some(e))
            || rows.iter().any(|s| s["example_id"] == r.example_id)
        {
            continue;
        }
        rows.push(json!({"example_id":r.example_id,"environment_id":r.environment_id,"observed_at":r.observed_at,"body_state":serde_json::to_value(&r.request.body).ok().and_then(|v|v.get("state").cloned()),"gaps":r.gaps,
  "request":action("read_request_material",q,json!({"example_id":r.example_id})),"response":action("read_response_material",q,json!({"example_id":r.example_id})),"context":action("read_source_evidence",q,json!({"example_id":r.example_id}))}));
    }
    rows.sort_by_key(|s| s["example_id"].to_string());
    rows
}
fn overview(q: &Query, summary: &EndpointSummary, k: &EndpointKnowledge) -> Value {
    let note = k.note.as_ref();
    json!({
        "project_id": q.project_id, "environment_id": q.environment_id, "knowledge_view": "official",
        "interface": {
            "facts": summary,
            "name": note.and_then(|n| n.name.as_ref()),
            "purpose": note.and_then(|n| n.purpose.as_deref()).map(preview),
            "directory_path": k.path.iter().map(|p| json!({"directory_id":p.id,"name":p.name})).collect::<Vec<_>>(),
            "documented_field_count": k.fields.len(),
            "field_count": Value::Null, "sample_count": Value::Null,
            "relation_count": k.links.iter().filter(|l| q.environment_id.is_none_or(|e|l.context.environment.is_none_or(|v|v==e))).count(),
            "retired_source_count": q.lifecycle.retired_source_count(k),
            "source_notice": if q.lifecycle.retired_source_count(k)>0 { Some("关联来源已废弃；当前接口仍按自身状态判断") } else { None },
            "raw_data_loaded": false,
            "unread": "Observed field/sample totals and captured URLs require explicit fields/samples/material reads; null is unknown, not zero."
        },
        "expand": [
            action("read_interface",q,json!({"section":"fields"})),
            action("read_interface",q,json!({"section":"relations"})),
            action("read_interface",q,json!({"section":"samples"})),
            action("read_source_evidence",q,json!({}))
        ],
        "full_description": action("read_interface",q,json!({"section":"description"}))
    })
}
fn relation_page(q: &Query, links: &[Link]) -> Result<Value, ReadError> {
    let id = q.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
    let direction = q.direction.as_deref().unwrap_or("both");
    if !matches!(direction, "incoming" | "outgoing" | "both") {
        return Err("INVALID_ARGUMENTS".into());
    }
    let mut selected: Vec<_> = links
        .iter()
        .filter(|l| {
            q.environment_id
                .is_none_or(|e| l.context.environment.is_none_or(|v| v == e))
        })
        .filter(|l| match direction {
            "incoming" => l.to == id,
            "outgoing" => l.from == id,
            _ => true,
        })
        .collect();
    selected.retain(|l| q.include_retired || !q.lifecycle.link_retired(l));
    selected.sort_by_key(|l| (q.lifecycle.link_retired(l), l.id.to_string()));
    page(
        "read_interface",
        q,
        selected.into_iter().map(|l| link_card(q, l)).collect(),
    )
}
impl Navigation {
    pub(super) async fn read(
        &self,
        q: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        let id = q.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
        let section = q.section.as_deref().unwrap_or("overview");
        if (q.field_key.is_some() && !matches!(section, "field" | "field_summary" | "relations"))
            || ((q.parent_key.is_some() || q.location.is_some()) && section != "fields")
            || (q.direction.is_some() && section != "relations")
            || (q.content_offset > 0
                && !matches!(section, "field" | "field_summary" | "description"))
            || (q.offset > 0
                && matches!(
                    section,
                    "overview" | "field" | "field_summary" | "description"
                ))
            || q.field_offset > 0
            || q.link_offset > 0
        {
            return Err("SECTION_ARGUMENT_MISMATCH".into());
        }
        let k = self
            .knowledge
            .endpoint(id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        if section == "overview" {
            let summary = summaries.iter().find(|s| s.id == id).ok_or("NOT_FOUND")?;
            return Ok(overview(q, summary, &k));
        }
        if section == "description" {
            return document("read_interface", q, json!({"endpoint_id":id,"note":k.note}));
        }
        if section == "relations" && q.field_key.is_none() {
            return relation_page(q, &k.links);
        }
        let f = self
            .endpoints
            .get(id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
            .ok_or("NOT_FOUND")?;
        let fields: Vec<_> = f
            .fields
            .iter()
            .filter(|f| field_visible(f, q.environment_id))
            .collect();
        let mut result = match section {
            "fields" => super::field_tree::read(q, &fields, &k)?,
            "field_summary" => {
                let key = q.field_key.as_deref().ok_or("FIELD_REQUIRED")?;
                let field = fields
                    .iter()
                    .find(|f| nexofolio_contracts::knowledge::field_note_key(f) == key)
                    .ok_or("FIELD_NOT_FOUND")?;
                super::field_summary::read(q, field, &k, summaries)?
            }
            "field" => {
                let key = q.field_key.as_deref().ok_or("FIELD_REQUIRED")?;
                let field = fields
                    .iter()
                    .find(|f| nexofolio_contracts::knowledge::field_note_key(f) == key)
                    .ok_or("FIELD_NOT_FOUND")?;
                let note = note_for(field, &k);
                let mut scoped_field = (*field).clone();
                if let Some(env) = q.environment_id {
                    scoped_field.labels.retain(|l| l.environment_id == env);
                }
                let mut scoped_note = note.cloned();
                if let Some(n) = &mut scoped_note {
                    n.context.evidence.retain(|e| {
                        q.environment_id
                            .is_none_or(|env| e.environment.is_none_or(|v| v == env))
                    });
                }
                let links = field_links(q, &k, field);
                let mut value = json!({"key":key,"facts":scoped_field,"knowledge":scoped_note,"relations":links.iter().map(|l|link_card(q,l)).collect::<Vec<_>>(),
     "text_lookup":action("search_interfaces",q,json!({"query":"","endpoint_id":Value::Null})),"evidence":action("read_source_evidence",q,json!({"field_key":key})),
     "note":"Textual mentions remain search clues; saved related evidence and links retain their original confirmation state."});
                if let Some(n) = note {
                    value["related_evidence"]=json!(n.context.evidence.iter().filter(|e|q.environment_id.is_none_or(|env|e.environment.is_none_or(|v|v==env))).map(|e|json!({"kind":e.kind,"description_preview":preview(&e.text),"endpoint_id":e.endpoint,"field":e.field,"environment_id":e.environment,"open":action("read_interface",q,json!({"endpoint_id":e.endpoint,"section":"overview"}))})).collect::<Vec<_>>());
                    if let Some(a) = n.context.observations.as_ref().and_then(|o| o.archive_id) {
                        value["values"] = action(
                            "read_field_values",
                            q,
                            json!({"archive_id":a,"field_key":key}),
                        );
                    }
                }
                document("read_interface", q, value)?
            }
            "relations" => {
                let mut links: Vec<_> = if let Some(key) = &q.field_key {
                    let field = fields
                        .iter()
                        .find(|f| nexofolio_contracts::knowledge::field_note_key(f) == *key)
                        .ok_or("FIELD_NOT_FOUND")?;
                    field_links(q, &k, field)
                } else {
                    k.links
                        .iter()
                        .filter(|l| {
                            q.environment_id
                                .is_none_or(|e| l.context.environment.is_none_or(|v| v == e))
                        })
                        .collect()
                };
                let direction = q.direction.as_deref().unwrap_or("both");
                if !matches!(direction, "incoming" | "outgoing" | "both") {
                    return Err("INVALID_ARGUMENTS".into());
                }
                links.retain(|l| match direction {
                    "incoming" => l.to == id,
                    "outgoing" => l.from == id,
                    _ => true,
                });
                links.retain(|l| q.include_retired || !q.lifecycle.link_retired(l));
                links.sort_by_key(|l| (q.lifecycle.link_retired(l), l.id.to_string()));
                page(
                    "read_interface",
                    q,
                    links.iter().map(|l| link_card(q, l)).collect(),
                )?
            }
            "samples" => page("read_interface", q, sample_cards(q, &f))?,
            _ => return Err("INVALID_ARGUMENTS".into()),
        };
        result["scope_note"] = json!(
            "Field types and meanings are project knowledge. Field labels, retained samples and scoped evidence must be checked against environment_id; unscoped legacy evidence remains unscoped."
        );
        Ok(result)
    }
}
