//! Per-tool bounded schemas. Descriptions identify the smallest appropriate read.
use rmcp::model::Tool;
use serde_json::{Value, json};
pub(super) fn names() -> Vec<(
    &'static str,
    &'static str,
    Vec<&'static str>,
    Vec<&'static str>,
)> {
    vec![
        (
            "list_projects",
            "Discover accessible projects by name. No project ID needed. Omit offset/limit for the first page (0/20); limit is 1-50, never 100. Page while has_more; ask on ambiguous names.",
            vec!["offset", "limit"],
            vec![],
        ),
        (
            "list_project_environments",
            "Discover configured environments of the selected project. Omit offset/limit for the first page (0/20); limit is 1-50. Use names; never infer environment from a domain.",
            vec!["project_id", "offset", "limit"],
            vec!["project_id"],
        ),
        (
            "browse_directory",
            "Explore saved business organization: section=directories lists direct children; section=interfaces lists direct members. Start at root without directory_id. Use unclassified=true for APIs without a folder. No fields or raw samples returned.",
            vec![
                "include_retired",
                "project_id",
                "environment_id",
                "directory_id",
                "section",
                "unclassified",
                "recursive",
                "offset",
                "limit",
                "revision",
            ],
            vec!["project_id"],
        ),
        (
            "search_interfaces",
            "Find interfaces by saved name/purpose/method/path, or search_in=fields by field key/label/meaning. Literal case-insensitive terms; no business-specific guessing. Field hits include owner identity and matched_field.open for a direct semantic read; no preliminary overview needed. Interface mode retains overview actions. No complete fields/samples loaded.",
            vec![
                "include_retired",
                "project_id",
                "environment_id",
                "directory_id",
                "recursive",
                "query",
                "search_in",
                "method",
                "business_scope",
                "offset",
                "limit",
                "revision",
            ],
            vec!["project_id"],
        ),
        (
            "read_interface",
            "Read an interface in small parts. Use overview to learn interface context; precise field hits can open field_summary directly. fields lists immediate children: key is the exact field identity, open reads semantics, expand exists only for children>0. field_summary reads COMPLETE semantic text with bounded references; field retains the full legacy document. relations filters saved links by field_key/direction; samples lists references. Omitting section preserves legacy output.",
            vec![
                "include_retired",
                "project_id",
                "environment_id",
                "endpoint_id",
                "section",
                "field_key",
                "parent_key",
                "location",
                "direction",
                "offset",
                "limit",
                "revision",
                "content_offset",
                "max_chars",
                "field_offset",
                "link_offset",
            ],
            vec!["project_id", "endpoint_id"],
        ),
        (
            "read_request_material",
            "Read a retained sample request: part=all|headers|url|body. Includes original values and completeness gaps. max_chars defaults to and cannot exceed 16384; normally omit it or copy next.arguments. Use content_offset/revision and concatenate JSON chunks. Captured authentication is historical, not verified current; never executes HTTP.",
            vec![
                "project_id",
                "environment_id",
                "endpoint_id",
                "example_id",
                "part",
                "source_record",
                "content_offset",
                "max_chars",
                "revision",
            ],
            vec!["project_id", "endpoint_id", "example_id"],
        ),
        (
            "read_response_material",
            "Read a retained sample response: part=all|headers|body. Preserves status, body state/root and original captured content. Missing response is explicit. Chunk continuation returns exact arguments; no HTTP execution.",
            vec![
                "project_id",
                "environment_id",
                "endpoint_id",
                "example_id",
                "part",
                "source_record",
                "content_offset",
                "max_chars",
                "revision",
            ],
            vec!["project_id", "endpoint_id", "example_id"],
        ),
        (
            "read_field_values",
            "Read typed observed values using a field observations.archive_id and field_key. Offset/limit list previews; value_id/content_offset read one complete original value. Observations are not a closed enum.",
            vec![
                "project_id",
                "environment_id",
                "endpoint_id",
                "archive_id",
                "field_key",
                "value_id",
                "offset",
                "limit",
                "content_offset",
                "max_chars",
            ],
            vec!["project_id", "endpoint_id", "archive_id", "field_key"],
        ),
        (
            "read_source_evidence",
            "Inspect provenance for exactly one field_key, link_id or retained example_id. Field/link evidence contains confirmed state and followable source references; sample evidence includes saved page/operation context. Chunk full evidence without generating new relationships.",
            vec![
                "project_id",
                "environment_id",
                "endpoint_id",
                "directory_id",
                "field_key",
                "link_id",
                "example_id",
                "source_record",
                "content_offset",
                "max_chars",
                "revision",
            ],
            vec!["project_id"],
        ),
        (
            "trace_dependencies",
            "Optional bounded traversal of saved input-source links for a selected interface and known_inputs/current BUSINESS authentication state. Returns gaps and cycles; observed links are not mandatory call order. Use read_interface relations for manual exploration.",
            vec![
                "project_id",
                "environment_id",
                "endpoint_id",
                "known_inputs",
                "authenticated",
                "max_depth",
                "max_steps",
            ],
            vec!["project_id", "endpoint_id"],
        ),
        (
            "inspect_knowledge_coverage",
            "Inspect saved coverage, not business completeness. section=summary provides grouped generation/confirmation/capture counts and expansion actions; section=issues with issue_kind reads the chosen group. field_key scopes one interface field. Choose the view for your task; no required read order. Omission preserves legacy issue pages. Never auto-curates or upgrades confirmation.",
            vec![
                "project_id",
                "environment_id",
                "directory_id",
                "endpoint_id",
                "recursive",
                "section",
                "issue_kind",
                "field_key",
                "offset",
                "limit",
                "revision",
            ],
            vec!["project_id"],
        ),
    ]
}
fn properties() -> Value {
    let entries = [
        ("project_id", json!({"type": "string", "format": "uuid"})),
        (
            "environment_id",
            json!({"type": "string", "format": "uuid"}),
        ),
        ("endpoint_id", json!({"type": "string", "format": "uuid"})),
        ("directory_id", json!({"type": "string", "format": "uuid"})),
        ("link_id", json!({"type": "string", "format": "uuid"})),
        ("archive_id", json!({"type": "string", "format": "uuid"})),
        (
            "offset",
            json!({"type": "integer", "minimum": 0, "default": 0}),
        ),
        (
            "limit",
            json!({"type": "integer", "minimum": 1, "maximum": 50, "default": 20, "description": "Page size: omit for 20 items, or choose 1-50. 100 is invalid. Follow the returned continuation for additional pages."}),
        ),
        (
            "revision",
            json!({"type": "string", "minLength": 64, "maxLength": 64, "description": "Echo the returned revision on continuation. Source changes require restarting that read."}),
        ),
        ("query", json!({"type": "string", "maxLength": 200})),
        (
            "search_in",
            json!({"enum": ["interface", "fields"], "default": "interface"}),
        ),
        ("section", json!({"type": "string"})),
        ("part", json!({"type": "string"})),
        ("recursive", json!({"type": "boolean", "default": false})),
        ("unclassified", json!({"type": "boolean", "default": false})),
        (
            "include_retired",
            json!({"type": "boolean", "default": false, "description":"Include retired history. Precise reads remain available by ID/key; retirement is independent of evidence confirmation."}),
        ),
        ("method", json!({"type": "string", "maxLength": 10})),
        (
            "business_scope",
            json!({"type": "string", "maxLength": 200}),
        ),
        (
            "field_key",
            json!({"type": "string", "maxLength": 4096, "description": "Exact location-qualified key returned by fields/search, e.g. request_body:customerId."}),
        ),
        (
            "parent_key",
            json!({"type": "string", "maxLength": 4096, "description": "Omit for root fields; pass the returned parent key to expand immediate children."}),
        ),
        (
            "location",
            json!({"type": "object", "required": ["in"], "properties": {"in": {"enum": ["path", "query", "request_header", "request_body", "response_header", "response_body"]}, "status": {"type": "integer", "minimum": 100, "maximum": 599}}, "additionalProperties": false}),
        ),
        (
            "issue_kind",
            json!({"enum":["interface_description_missing","unclassified","field_description_missing","field_generation_incomplete","field_meaning_unconfirmed","request_material_not_retained","capture_gap"]}),
        ),
        (
            "direction",
            json!({"enum": ["incoming", "outgoing", "both"], "default": "both"}),
        ),
        (
            "field_offset",
            json!({"type": "integer", "minimum": 0, "maximum": 100000}),
        ),
        (
            "link_offset",
            json!({"type": "integer", "minimum": 0, "maximum": 100000}),
        ),
        (
            "content_offset",
            json!({"type": "integer", "minimum": 0, "default": 0}),
        ),
        (
            "max_chars",
            json!({"type": "integer", "minimum": 256, "maximum": 16384, "default": 16384, "description": "Unicode characters per document chunk, 256-16384. Normally omit for 16384; small chunks are for targeted reads/testing. Continue with exact next.arguments."}),
        ),
        ("example_id", json!({"type": "string", "maxLength": 100})),
        ("value_id", json!({"type": "string", "maxLength": 64})),
        (
            "source_record",
            json!({"type": "object", "required": ["batch_id", "record_id"], "properties": {"batch_id": {"type": "string", "format": "uuid"}, "record_id": {"type": "string", "format": "uuid"}}, "additionalProperties": false}),
        ),
        (
            "authenticated",
            json!({"type": "boolean", "default": false}),
        ),
        (
            "known_inputs",
            json!({"type": "array", "maxItems": 100, "items": {"type": "object", "required": ["endpoint", "location", "path"], "properties": {"endpoint": {"type": "string", "format": "uuid"}, "location": {"type": "object", "required": ["in"], "properties": {"in": {"enum": ["path", "query", "request_header", "request_body"]}}, "additionalProperties": false}, "path": {"type": "string", "maxLength": 500}}, "additionalProperties": false}}),
        ),
        (
            "max_depth",
            json!({"type": "integer", "minimum": 1, "maximum": 8, "default": 4}),
        ),
        (
            "max_steps",
            json!({"type": "integer", "minimum": 1, "maximum": 100, "default": 60}),
        ),
    ];
    Value::Object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

fn schema(name: &str, keys: &[&str], required: &[&str]) -> Value {
    let all = properties();
    let mut props = serde_json::Map::new();
    for k in keys {
        props.insert((*k).into(), all[*k].clone());
    }
    if name == "browse_directory" {
        props.insert(
            "section".into(),
            json!({"enum":["directories","interfaces"],"default":"directories"}),
        );
    }
    if name == "read_interface" {
        props.insert(
            "section".into(),
            json!({"enum":["overview","description","fields","field_summary","field","relations","samples"]}),
        );
    }
    if name == "read_request_material" {
        props.insert(
            "part".into(),
            json!({"enum":["all","headers","url","body"],"default":"all"}),
        );
    }
    if name == "read_response_material" {
        props.insert(
            "part".into(),
            json!({"enum":["all","headers","body"],"default":"all"}),
        );
    }
    if name == "inspect_knowledge_coverage" {
        props.insert("section".into(), json!({"enum":["summary","issues"]}));
    }
    // New tool discovery requires a small explicit view. Runtime validation
    // still accepts omission for clients using the previous cached schema.
    let public_required = if name == "read_interface" {
        vec!["project_id", "endpoint_id", "section"]
    } else {
        required.to_vec()
    };
    json!({"type":"object","properties":props,"required":public_required,"additionalProperties":false})
}
pub(super) fn definitions() -> Vec<Tool> {
    names()
        .into_iter()
        .map(|(name, description, keys, required)| {
            Tool::new(
                name,
                description,
                schema(name, &keys, &required).as_object().unwrap().clone(),
            )
        })
        .collect()
}
pub(super) fn validate_keys(tool: &str, args: &Value) -> Result<(), &'static str> {
    let (name, _, keys, required) = names()
        .into_iter()
        .find(|t| t.0 == tool)
        .ok_or("UNKNOWN_TOOL")?;
    let object = args.as_object().ok_or("INVALID_ARGUMENTS")?;
    if object.keys().any(|k| !keys.contains(&k.as_str()))
        || required
            .iter()
            .any(|k| object.get(*k).is_none_or(Value::is_null))
    {
        return Err("INVALID_ARGUMENTS");
    }
    let s = schema(name, &keys, &required);
    for (k, v) in object {
        if let Some(options) = s["properties"][k]["enum"].as_array()
            && !options.contains(v)
        {
            return Err("INVALID_ARGUMENTS");
        }
    }
    Ok(())
}
pub(super) fn retain_arguments(tool: &str, args: &mut Value) {
    if let Some((_, _, keys, _)) = names().into_iter().find(|t| t.0 == tool) {
        args.as_object_mut()
            .expect("arguments")
            .retain(|k, v| keys.contains(&k.as_str()) && !v.is_null());
    }
}
