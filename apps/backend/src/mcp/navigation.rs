//! Knowledge navigation application reads only public ports. No model or raw sample retrieval.
use nexofolio_access_contracts::{McpPrincipal, McpProjectAccess};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{
    endpoint::EndpointReader,
    knowledge::{InputRef, KnowledgeReader},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct Navigation {
    pub(crate) access: Arc<dyn McpProjectAccess>,
    pub(crate) endpoints: Arc<dyn EndpointReader>,
    pub(crate) knowledge: Arc<dyn KnowledgeReader>,
}
impl Navigation {
    pub fn new(
        access: Arc<dyn McpProjectAccess>,
        endpoints: Arc<dyn EndpointReader>,
        knowledge: Arc<dyn KnowledgeReader>,
    ) -> Self {
        Self {
            access,
            endpoints,
            knowledge,
        }
    }
    pub(crate) async fn call(
        &self,
        principal: &McpPrincipal,
        tool: &str,
        args: Value,
    ) -> Result<Value, &'static str> {
        if matches!(tool, "list_projects" | "list_project_environments") {
            return self.discover(principal, tool, args).await;
        }
        let request: Query = serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
        if request.limit == 0
            || request.limit > 50
            || (tool != "read_field_values" && request.offset > 100_000)
            || request.field_offset > 100_000
            || request.link_offset > 100_000
            || request.query.chars().count() > 200
            || request
                .business_scope
                .as_ref()
                .is_some_and(|s| s.chars().count() > 200)
            || request.method.as_ref().is_some_and(|s| s.len() > 10)
            || request.known_inputs.len() > 100
            || !(1..=8).contains(&request.max_depth)
            || !(1..=100).contains(&request.max_steps)
        {
            return Err("INVALID_BOUNDS");
        }
        self.access
            .require_mcp_project(principal, request.project_id)
            .await
            .map_err(|_| "PROJECT_ACCESS_DENIED")?;
        let summaries = self
            .endpoints
            .list(request.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        if let Some(id) = request.endpoint_id
            && !summaries.iter().any(|s| s.id == id)
        {
            return Err("ENDPOINT_NOT_IN_PROJECT");
        }
        match tool {
            "search_interfaces" => {
                let notes = self
                    .knowledge
                    .notes(request.project_id)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                let query = request.query.to_lowercase();
                let mut items = Vec::new();
                for summary in summaries {
                    if request
                        .method
                        .as_ref()
                        .is_some_and(|method| !summary.method.eq_ignore_ascii_case(method))
                    {
                        continue;
                    }
                    if request.business_scope.as_ref().is_some_and(|scope| {
                        !summary
                            .path_template
                            .to_lowercase()
                            .contains(&scope.to_lowercase())
                    }) {
                        continue;
                    }
                    if request.environment_id.is_some_and(|env| {
                        !summary.environments.iter().any(|e| e.environment_id == env)
                    }) {
                        continue;
                    }
                    let note = notes.iter().find(|n| n.endpoint == summary.id);
                    let haystack = format!(
                        "{} {} {} {}",
                        summary.method,
                        summary.path_template,
                        note.and_then(|n| n.name.as_deref()).unwrap_or(""),
                        note.and_then(|n| n.purpose.as_deref()).unwrap_or("")
                    )
                    .to_lowercase();
                    if !matches_goal(&query, &haystack) {
                        continue;
                    }
                    items.push(json!({"endpoint_id":summary.id,"method":summary.method,"path":summary.path_template,
                        "name":note.and_then(|n| n.name.as_ref()),"purpose":note.and_then(|n| n.purpose.as_ref()),
                        "environments":summary.environments}));
                }
                items.sort_by_key(|v| v["endpoint_id"].to_string());
                let total = items.len();
                let items: Vec<_> = items
                    .into_iter()
                    .skip(request.offset)
                    .take(request.limit)
                    .collect();
                Ok(
                    json!({"items":items,"total":total,"offset":request.offset,"limit":request.limit,
                    "has_more":request.offset+request.limit<total}),
                )
            }
            "read_field_values" => {
                let q = nexofolio_contracts::field_values::FieldValueQuery {
                    project: request.project_id,
                    endpoint: request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?,
                    archive: request.archive_id.ok_or("ARCHIVE_REQUIRED")?,
                    field: request.field_key.clone().ok_or("FIELD_REQUIRED")?,
                    offset: request.offset,
                    limit: request.limit,
                    environment: request.environment_id,
                };
                if let Some(value_id) = request.value_id {
                    let value = self
                        .knowledge
                        .observed_value(q, &value_id)
                        .await
                        .map_err(|_| "VALUE_UNAVAILABLE")?;
                    let encoded = serde_json::to_string(&value).map_err(|_| "VALUE_UNAVAILABLE")?;
                    let total = encoded.chars().count();
                    if request.content_offset > total {
                        return Err("INVALID_BOUNDS");
                    }
                    let data: String = encoded
                        .chars()
                        .skip(request.content_offset)
                        .take(16384)
                        .collect();
                    let next = request.content_offset + data.chars().count();
                    Ok(
                        json!({"archive_id":request.archive_id,"value_id":value_id,"encoding":"json_text_chunks","data":data,"next_offset":next,"total_chars":total,"has_more":next<total}),
                    )
                } else {
                    self.knowledge
                        .observed_values(q)
                        .await
                        .map_err(|_| "VALUE_UNAVAILABLE")
                        .and_then(|v| serde_json::to_value(v).map_err(|_| "VALUE_UNAVAILABLE"))
                }
            }
            "read_request_material" => {
                let endpoint = request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
                let example = request.example_id.as_deref().ok_or("EXAMPLE_REQUIRED")?;
                let raw = self
                    .endpoints
                    .example(endpoint, example)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                    .ok_or("SAMPLE_NOT_RETAINED")?;
                let observation = serde_json::from_value(raw).map_err(|_| "REQUEST_UNREADABLE")?;
                let material = nexofolio_contracts::request::RequestMaterial::from_observation(
                    example,
                    &observation,
                )
                .ok_or("REQUEST_UNREADABLE")?;
                if request
                    .environment_id
                    .is_some_and(|e| Some(e) != material.environment_id)
                {
                    return Err("ENVIRONMENT_MISMATCH");
                }
                if request.source_record.is_some()
                    && request.source_record != material.observation_key
                {
                    return Err("SOURCE_CHANGED_RESTART_READ");
                }
                let source_record = material.observation_key;
                let encoded = serde_json::to_string(&material).map_err(|_| "REQUEST_UNREADABLE")?;
                let total = encoded.chars().count();
                if request.content_offset > total {
                    return Err("INVALID_BOUNDS");
                }
                let data: String = encoded
                    .chars()
                    .skip(request.content_offset)
                    .take(16_384)
                    .collect();
                let next = request.content_offset + data.chars().count();
                Ok(
                    json!({"example_id":example,"encoding":"json_text_chunks","source_record":source_record,"data":data,"offset":request.content_offset,"next_offset":next,"total_chars":total,"has_more":next<total,"instruction":"Concatenate data chunks from the same source_record before parsing; pass source_record on continuation requests. Captured authentication is historical and unverified; supply current business credentials separately. No HTTP call was executed."}),
                )
            }
            "read_interface" => {
                let endpoint = request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
                let facts = self
                    .endpoints
                    .get(endpoint)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                    .ok_or("NOT_FOUND")?;
                let knowledge = self
                    .knowledge
                    .endpoint(endpoint)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                let fields: Vec<_> = facts.fields.iter().skip(request.field_offset).take(request.limit).map(|f| {
                    let key = nexofolio_contracts::knowledge::field_note_key(f);
                    let note = knowledge.fields.iter().find(|n| n.path == key || (n.context.location.is_none() && n.path == f.path.to_string()));
                    json!({"key":key,"path":f.path,"location":f.location,"types":f.types,"labels":f.labels,
                        "knowledge":note})
                }).collect();
                let links: Vec<_> = knowledge
                    .links
                    .iter()
                    .skip(request.link_offset)
                    .take(request.limit)
                    .collect();
                let materials: Vec<_> = facts
                    .requests
                    .iter()
                    .filter(|r| {
                        request
                            .environment_id
                            .is_none_or(|e| r.environment_id == Some(e))
                    })
                    .collect();
                let material_page: Vec<_> = materials.iter().skip(request.offset).take(request.limit).map(|r| json!({
                    "example_id":r.example_id,"environment_id":r.environment_id,"observed_at":r.observed_at,
                    "body_state":serde_json::to_value(&r.request.body).ok().and_then(|b| b.get("state").cloned()),
                    "headers_complete":r.request.headers.as_ref().map(|h| h.complete),"gaps":r.gaps,
                    "read_with":"read_request_material"
                })).collect();
                Ok(
                    json!({"facts":facts.summary,"addresses":facts.addresses,"note":knowledge.note,
                    "fields":fields,"field_total":facts.fields.len(),"field_offset":request.field_offset,
                    "fields_has_more":request.field_offset+request.limit<facts.fields.len(),
                    "links":links,"link_total":knowledge.links.len(),"link_offset":request.link_offset,
                    "links_has_more":request.link_offset+request.limit<knowledge.links.len(),
                    "request_materials":material_page,"request_material_total":materials.len(),
                    "request_material_has_more":request.offset+request.limit<materials.len(),
                    "request_material_offset":request.offset,"raw_samples_returned":false}),
                )
            }
            "trace_dependencies" => {
                let endpoint = request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
                let target = self
                    .endpoints
                    .get(endpoint)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                    .ok_or("NOT_FOUND")?;
                let target_knowledge = self
                    .knowledge
                    .endpoint(endpoint)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                let environment = match nexofolio_curate::trace_environment(
                    &target,
                    Some(&target_knowledge),
                    request.environment_id,
                ) {
                    Ok(environment) => environment,
                    Err(_) => {
                        return serde_json::to_value(nexofolio_curate::trace_upstream(
                            endpoint,
                            &[target],
                            &[(endpoint, target_knowledge)],
                            nexofolio_curate::TraceOptions {
                                environment: request.environment_id,
                                known_inputs: &request.known_inputs,
                                authenticated: request.authenticated,
                                max_depth: request.max_depth,
                                max_steps: request.max_steps,
                            },
                        ))
                        .map_err(|_| "KNOWLEDGE_UNAVAILABLE");
                    }
                };
                // Load a bounded reachable graph, never the entire project field tree.
                let mut pending: Vec<_> = target_knowledge
                    .links
                    .iter()
                    .filter(|link| link.to == endpoint && link.context.environment == environment)
                    .map(|link| link.from)
                    .collect();
                let mut graph_truncated = false;
                let mut facts = vec![target];
                let mut knowledge = vec![(endpoint, target_knowledge)];
                while let Some(id) = pending.pop() {
                    if knowledge.iter().any(|(seen, _)| *seen == id) {
                        continue;
                    }
                    if knowledge.len() >= 32 {
                        graph_truncated = true;
                        break;
                    }
                    if !summaries.iter().any(|s| s.id == id) {
                        continue;
                    }
                    let Some(fact) = self
                        .endpoints
                        .get(id)
                        .await
                        .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                    else {
                        continue;
                    };
                    let info = self
                        .knowledge
                        .endpoint(id)
                        .await
                        .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                    for link in &info.links {
                        if link.to == id && link.context.environment == environment {
                            pending.push(link.from);
                        }
                    }
                    facts.push(fact);
                    knowledge.push((id, info));
                }
                let mut result = nexofolio_curate::trace_upstream(
                    endpoint,
                    &facts,
                    &knowledge,
                    nexofolio_curate::TraceOptions {
                        environment,
                        known_inputs: &request.known_inputs,
                        authenticated: request.authenticated,
                        max_depth: request.max_depth,
                        max_steps: request.max_steps,
                    },
                );
                result.truncated |= graph_truncated || !pending.is_empty();
                serde_json::to_value(result).map_err(|_| "KNOWLEDGE_UNAVAILABLE")
            }
            _ => Err("UNKNOWN_TOOL"),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    project_id: ProjectId,
    #[serde(default)]
    archive_id: Option<uuid::Uuid>,
    #[serde(default)]
    field_key: Option<String>,
    #[serde(default)]
    value_id: Option<String>,
    #[serde(default)]
    example_id: Option<String>,
    #[serde(default)]
    content_offset: usize,
    #[serde(default)]
    source_record: Option<nexofolio_contracts::observation::ObservationKey>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    business_scope: Option<String>,
    #[serde(default)]
    endpoint_id: Option<EndpointId>,
    #[serde(default)]
    environment_id: Option<EnvironmentId>,
    #[serde(default)]
    query: String,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    field_offset: usize,
    #[serde(default)]
    link_offset: usize,
    #[serde(default = "limit")]
    limit: usize,
    #[serde(default)]
    known_inputs: Vec<InputRef>,
    #[serde(default)]
    authenticated: bool,
    #[serde(default = "depth")]
    max_depth: usize,
    #[serde(default = "steps")]
    max_steps: usize,
}
fn limit() -> usize {
    20
}
fn depth() -> usize {
    4
}
fn steps() -> usize {
    60
}
fn matches_goal(query: &str, haystack: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    if haystack.contains(query) {
        return true;
    }
    if query.contains("技术咨询") && !haystack.contains("技术咨询") && !haystack.contains("consult")
    {
        return false;
    }
    if query.contains("技术咨询") && (query.contains("创建") || query.contains("新增")) {
        return ["新增", "创建", "保存并提交", "create", "saveandsubmit"]
            .iter()
            .any(|word| haystack.contains(word));
    }
    if query.contains("技术咨询") && query.contains("详情") {
        return ["详情", "detail", "viewconsult"]
            .iter()
            .any(|word| haystack.contains(word));
    }
    if query.contains("技术咨询") && (query.contains("处理") || query.contains("完成")) {
        return ["处理", "完成", "completetask"]
            .iter()
            .any(|word| haystack.contains(word));
    }
    query.split_whitespace().all(|word| haystack.contains(word))
}
