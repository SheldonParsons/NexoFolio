//! Small state annotations, independent of semantic confirmation and raw evidence.
use super::*;
use nexofolio_contracts::knowledge::{DocumentLifecycle, Retirement};
fn status(decision: Option<&Retirement>) -> Value {
    match decision {
        Some(d) => json!({"state":"retired","decision_version":d.version,"target":d.target}),
        None => json!({"state":"active"}),
    }
}
pub(super) fn annotate(value: &mut Value, q: &Query, inherited: Option<EndpointId>) {
    let Value::Object(object) = value else {
        if let Value::Array(items) = value {
            for item in items {
                annotate(item, q, inherited);
            }
        }
        return;
    };
    // Tool arguments must stay executable and schema-valid.
    if object.contains_key("tool") && object.contains_key("arguments") {
        return;
    }
    // Full relation evidence and the legacy projection retain the original
    // Link shape. Annotate those too, without changing its confirmation.
    if object.contains_key("from")
        && object.contains_key("to")
        && object.contains_key("id")
        && let Ok(link) = serde_json::from_value::<nexofolio_contracts::knowledge::Link>(
            Value::Object(object.clone()),
        )
    {
        let retired = q.lifecycle.link_retired(&link);
        object.insert(
            "lifecycle".into(),
            json!({"state": if retired { "retired_source" } else { "active" }}),
        );
        if retired {
            object.insert("source_notice".into(), json!("关联来源已废弃"));
        }
    }
    let endpoint = object
        .get("endpoint_id")
        .or_else(|| object.get("endpoint"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .or(inherited);
    let directory = object
        .get("directory_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok());
    let field = object
        .get("field_key")
        .or_else(|| object.get("key"))
        .and_then(Value::as_str);
    let decision = if let Some(folder) = directory {
        q.lifecycle.folder(folder)
    } else if let (Some(id), Some(key)) = (endpoint, field) {
        q.lifecycle.field_key(id, key)
    } else {
        endpoint.and_then(|id| q.lifecycle.endpoint(id))
    };
    let identity = directory.is_some() || object.contains_key("endpoint_id") || field.is_some();
    if let (Some(id), Some(path)) = (endpoint, object.get("field").and_then(Value::as_str)) {
        let location = object
            .get("source")
            .and_then(|s| s.get("location"))
            .or_else(|| object.get("location"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        if q.lifecycle.source_field(id, location, path).is_some() {
            object.insert("source_notice".into(), json!("关联来源已废弃"));
        }
    }
    if identity {
        object.insert("lifecycle".into(), status(decision));
    }
    for (key, child) in object.iter_mut() {
        if !matches!(
            key.as_str(),
            "lifecycle"
                | "body"
                | "body_root"
                | "raw"
                | "sample"
                | "observation"
                | "context"
                | "options"
        ) {
            annotate(child, q, endpoint);
        }
    }
}
pub(crate) fn summary(lifecycle: &DocumentLifecycle) -> Value {
    json!({"version":lifecycle.version,"reconciled_version":lifecycle.reconciled_version,"retired_count":lifecycle.direct_count(),"pending_reconciliation":lifecycle.pending(),"notice":if lifecycle.pending(){Some("存在废弃变更，当前整理尚未重新核对")}else{None}})
}
pub(super) fn envelope(result: &mut Value, q: &Query) {
    if q.lifecycle.is_empty() {
        return;
    }
    annotate(result, q, q.endpoint_id);
    result["project_lifecycle"] = summary(&q.lifecycle);
    let target = q
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
    result["target_lifecycle"] = status(target);
    result["retired_directories"] = super::projection::action(
        "browse_directory",
        q,
        json!({"section":"directories","include_retired":true,"endpoint_id":Value::Null}),
    );
    result["retired_history"] = super::projection::action(
        "search_interfaces",
        q,
        json!({"endpoint_id":Value::Null,"query":"","include_retired":true}),
    );
    if q.endpoint_id.is_some() {
        result["retired_relations"] = super::projection::action(
            "read_interface",
            q,
            json!({"section":"relations","include_retired":true}),
        );
        result["expand_retired_fields"] = super::projection::action(
            "read_interface",
            q,
            json!({"section":"fields","include_retired":true}),
        );
    }
    if !q.include_retired
        && let Some(next) = result.get_mut("all_interfaces")
    {
        next["arguments"]["include_retired"] = json!(true);
    }
}
pub(super) fn visible(q: &Query, endpoint: EndpointId, field: Option<&str>) -> bool {
    q.include_retired
        || field.map_or_else(
            || q.lifecycle.endpoint(endpoint).is_none(),
            |key| q.lifecycle.field_key(endpoint, key).is_none(),
        )
}
