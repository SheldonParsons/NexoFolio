//! Explicit bounded traversal over saved relations; no business inference.
use super::*;
impl Navigation {
    pub(super) async fn trace(
        &self,
        request: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        let endpoint = request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
        let target = self
            .endpoints
            .get(endpoint)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
            .ok_or("NOT_FOUND")?;
        let mut target_knowledge = self
            .knowledge
            .endpoint(endpoint)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let retired_relations = target_knowledge
            .links
            .iter()
            .filter(|l| request.lifecycle.link_retired(l))
            .count();
        target_knowledge
            .links
            .retain(|l| !request.lifecycle.link_retired(l));
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
                .map_err(|_| ReadError::from("KNOWLEDGE_UNAVAILABLE"));
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
            let mut info = self
                .knowledge
                .endpoint(id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
            info.links.retain(|l| !request.lifecycle.link_retired(l));
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
        let mut value =
            serde_json::to_value(result).map_err(|_| ReadError::from("KNOWLEDGE_UNAVAILABLE"))?;
        if retired_relations > 0 {
            value["retired_source_relations"] = json!(retired_relations);
            value["source_notice"] = json!("关联来源已废弃；可读取 relations 历史自行核对");
        }
        Ok(value)
    }
}
