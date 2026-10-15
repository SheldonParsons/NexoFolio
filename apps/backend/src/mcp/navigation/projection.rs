//! Bounded projections with explicit source revisions and continuation calls.
use super::*;
use sha2::{Digest, Sha256};
pub(super) fn preview(s: &str) -> String {
    s.chars().take(320).collect()
}
pub(super) fn action(tool: &str, q: &Query, extra: Value) -> Value {
    let mut args = json!({"project_id":q.project_id});
    if let Some(e) = q.environment_id {
        args["environment_id"] = json!(e);
    }
    if let Some(e) = q.endpoint_id {
        args["endpoint_id"] = json!(e);
    }
    if q.include_retired {
        args["include_retired"] = json!(true);
    }
    for (k, v) in extra.as_object().expect("action properties") {
        args[k] = v.clone();
    }
    super::super::tools::retain_arguments(tool, &mut args);
    json!({"tool":tool,"arguments":args})
}
fn revision(q: &Query, tool: &str, value: &Value) -> Result<String, &'static str> {
    let mut scope = q.clone();
    scope.offset = 0;
    scope.content_offset = 0;
    scope.revision = None;
    scope.source_record = None;
    scope.limit = 20;
    scope.max_chars = 16384;
    let bytes = serde_json::to_vec(&(tool, scope, q.lifecycle.version, value))
        .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
    let rev: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if q.revision.as_ref().is_some_and(|r| r != &rev) {
        return Err("SOURCE_CHANGED_RESTART_READ");
    }
    Ok(rev)
}
pub(super) fn page(tool: &str, q: &Query, items: Vec<Value>) -> Result<Value, ReadError> {
    page_in_context(tool, q, items, &Value::Null)
}

// Paging preserves object identities and saved meaning, not a freeze of live
// traffic counters. Complete documents below still hash every original value.
fn stable_paging_facts(mut value: Value) -> Value {
    fn visit(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.retain(|key, _| {
                    !matches!(
                        key.as_str(),
                        "calls" | "last_seen" | "same_structure_calls" | "retained_samples"
                    )
                });
                for value in object.values_mut() {
                    visit(value);
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item);
                }
            }
            _ => {}
        }
    }
    visit(&mut value);
    value
}

pub(super) fn page_in_context(
    tool: &str,
    q: &Query,
    mut items: Vec<Value>,
    context: &Value,
) -> Result<Value, ReadError> {
    if !q.lifecycle.is_empty() {
        for item in &mut items {
            super::lifecycle::annotate(item, q, q.endpoint_id);
        }
    }
    let rev = revision(
        q,
        tool,
        &json!({"items":stable_paging_facts(json!(items)),"context":context}),
    )?;
    let total = items.len();
    page_result(
        tool,
        q,
        items.into_iter().skip(q.offset).take(q.limit).collect(),
        total,
        rev,
    )
}

pub(super) fn paged(
    tool: &str,
    q: &Query,
    mut items: Vec<Value>,
    total: usize,
    source: &Value,
) -> Result<Value, ReadError> {
    for item in &mut items {
        if !q.lifecycle.is_empty() {
            super::lifecycle::annotate(item, q, q.endpoint_id);
        }
    }
    page_result(tool, q, items, total, revision(q, tool, source)?)
}

fn page_result(
    tool: &str,
    q: &Query,
    selected: Vec<Value>,
    total: usize,
    rev: String,
) -> Result<Value, ReadError> {
    if q.offset > total {
        return Err(ReadError::bounds("offset", q.offset, 0, total, 0, "items"));
    }
    let next = q.offset + selected.len();
    let mut args = q.arguments();
    args["offset"] = json!(next);
    args["revision"] = json!(rev);
    super::super::tools::retain_arguments(tool, &mut args);
    Ok(
        json!({"project_id":q.project_id,"environment_id":q.environment_id,"knowledge_view":"official",
 "items":selected,"total":total,"offset":q.offset,"limit":q.limit,"revision":rev,"has_more":next<total,
 "next_offset":if next<total {Some(next)}else{None},"next":if next<total {json!({"tool":tool,"arguments":args})}else{Value::Null}}),
    )
}
pub(super) fn document(tool: &str, q: &Query, mut value: Value) -> Result<Value, ReadError> {
    if !q.lifecycle.is_empty() && tool == "read_source_evidence" && value.is_object() {
        let decision = q
            .directory_id
            .and_then(|id| q.lifecycle.folder(id))
            .or_else(|| {
                q.endpoint_id.and_then(|id| {
                    q.field_key.as_ref().map_or_else(
                        || q.lifecycle.endpoint(id),
                        |key| q.lifecycle.field_key(id, key),
                    )
                })
            });
        value["retirement"] = json!(decision);
    }
    if !q.lifecycle.is_empty()
        && (tool == "read_interface" || (tool == "read_source_evidence" && q.example_id.is_none()))
    {
        super::lifecycle::annotate(&mut value, q, q.endpoint_id);
    }
    let rev = revision(q, tool, &value)?;
    let text = serde_json::to_string(&value).map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
    let total = text.chars().count();
    if q.content_offset > total {
        return Err(ReadError::bounds(
            "content_offset",
            q.content_offset,
            0,
            total,
            0,
            "characters",
        ));
    }
    let data: String = text
        .chars()
        .skip(q.content_offset)
        .take(q.max_chars)
        .collect();
    let next = q.content_offset + data.chars().count();
    let mut args = q.arguments();
    args["content_offset"] = json!(next);
    args["revision"] = json!(rev);
    super::super::tools::retain_arguments(tool, &mut args);
    Ok(
        json!({"project_id":q.project_id,"environment_id":q.environment_id,"endpoint_id":q.endpoint_id,"example_id":q.example_id,
 "source_record":value.get("observation_key"),"revision":rev,"encoding":"json_text_chunks","data":data,"offset":q.content_offset,
 "next_offset":next,"total_chars":total,"has_more":next<total,"next":if next<total {json!({"tool":tool,"arguments":args})}else{Value::Null},
 "instruction":"Concatenate data from the same revision before parsing JSON. No business HTTP request was executed."}),
    )
}
