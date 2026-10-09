//! Explicit bounded tool schemas; application validation also checks every request.
use rmcp::model::Tool;
use serde_json::json;
pub(super) fn definitions() -> Vec<Tool> {
    let common = json!({
        "project_id":{"type":"string","format":"uuid"},
        "endpoint_id":{"type":"string","format":"uuid"},
        "archive_id":{"type":"string","format":"uuid"},
        "field_key":{"type":"string","maxLength":4096},
        "value_id":{"type":"string","maxLength":64},
        "example_id":{"type":"string","maxLength":100},
        "content_offset":{"type":"integer","minimum":0},
        "source_record":{"type":"object","required":["batch_id","record_id"],"additionalProperties":false,"properties":{"batch_id":{"type":"string","format":"uuid"},"record_id":{"type":"string","format":"uuid"}}},
        "environment_id":{"type":"string","format":"uuid","description":"Trace infers the target's sole known environment when omitted; multiple environments return environment_required, never a mixed graph"},
        "query":{"type":"string","maxLength":200},
        "method":{"type":"string","maxLength":10},
        "business_scope":{"type":"string","maxLength":200,"description":"Filter observed API path, e.g. /client/ or /manager/; do not infer permission from this scope"},
        "offset":{"type":"integer","minimum":0},
        "field_offset":{"type":"integer","minimum":0,"maximum":100000},
        "link_offset":{"type":"integer","minimum":0,"maximum":100000},
        "limit":{"type":"integer","minimum":1,"maximum":50,"default":20},
        "authenticated":{"type":"boolean","default":false,"description":"Caller declares current valid BUSINESS authentication; never pass a captured token"},
        "known_inputs":{"type":"array","maxItems":100,"items":{"type":"object","required":["endpoint","location","path"],
            "properties":{"endpoint":{"type":"string","format":"uuid"},"location":{"type":"object","required":["in"],"properties":{"in":{"enum":["path","query","request_header","request_body"]}}},"path":{"type":"string","maxLength":500}}}},
        "max_depth":{"type":"integer","minimum":1,"maximum":8,"default":4},
        "max_steps":{"type":"integer","minimum":1,"maximum":100,"default":60}
    });
    let mut tools = vec![
        Tool::new("list_projects", "Start here: list only projects currently accessible to this MCP token's owner. No project_id needed. Returns project_id, name and status; follow next_offset while has_more=true. Resolve the user's project by name; ask when ambiguous.",
            json!({"type":"object","properties":{"offset":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":50,"default":20}},"additionalProperties":false}).as_object().unwrap().clone()),
        Tool::new("list_project_environments", "List configured environments for an authorized project from list_projects. Returns project_id, environment_id and name. Follow next_offset while has_more=true; use the chosen environment_id in subsequent tools. An environment may have no captured interfaces yet; do not infer test/production from IDs.",
            json!({"type":"object","properties":{"project_id":{"type":"string","format":"uuid"},"offset":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":50,"default":20}},"required":["project_id"],"additionalProperties":false}).as_object().unwrap().clone()),
    ];
    tools.extend([
        ("search_interfaces", "Find a goal in saved API names/purposes and method/path within authorized project/environment. Offset paging; no live model.", vec!["project_id"]),
        ("read_interface", "Read saved facts, field meanings, names, source references and directed relations. Page fields/links independently. Includes references to complete captured HTTP request material; captured credentials are historical, not current authentication.", vec!["project_id","endpoint_id"]),
        ("read_request_material", "Read original captured request headers, URL/query, body state/root/content, response headers and provenance. Use content_offset until has_more=false, concatenate data and parse JSON. Historical credentials are not verified current. Does not execute HTTP calls.", vec!["project_id","endpoint_id","example_id"]),
        ("read_field_values", "Read complete typed values from a field knowledge observations.archive_id. Field key includes location. Offset/limit page previews; value_id and content_offset retrieve the original JSON value in chunks. Preserve type, source and capture gaps; values are not a closed business enum.", vec!["project_id","endpoint_id","archive_id","field_key"]),
        ("trace_dependencies", "Trace missing input sources, considering known inputs and business authentication; return references, gaps and cycles. Observed usage is not a hard prerequisite.", vec!["project_id","endpoint_id"]),
    ].into_iter().map(|(name, description, required)| Tool::new(name, description,
        json!({"type":"object","properties":common,"required":required,"additionalProperties":false}).as_object().unwrap().clone())));
    tools
}
