//! Authorized read application. Public read ports only: no SQL, models or writes.
use nexofolio_access_contracts::{McpPrincipal, McpProjectAccess};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{
    endpoint::{EndpointReader, EndpointSummary},
    knowledge::{InputRef, KnowledgeReader},
};
use serde_json::{Value, json};
use std::sync::Arc;
mod catalogue;
mod coverage;
mod error;
mod field_summary;
mod field_tree;
mod interface;
mod legacy;
pub(super) mod lifecycle;
mod material;
mod projection;
mod query;
mod search;
mod source_actions;
mod trace;
mod values;
pub(crate) use error::ReadError;
use query::Query;
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
    ) -> Result<Value, ReadError> {
        super::tools::validate_keys(tool, &args)?;
        if matches!(tool, "list_projects" | "list_project_environments") {
            return self.discover(principal, tool, args).await;
        }
        let mut q: Query = serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
        q.validate()?;
        self.access
            .require_mcp_project(principal, q.project_id)
            .await
            .map_err(|_| "PROJECT_ACCESS_DENIED")?;
        q.lifecycle = self
            .knowledge
            .lifecycle(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let summaries = self
            .endpoints
            .list(q.project_id)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        if let Some(id) = q.endpoint_id
            && !summaries.iter().any(|s| s.id == id)
        {
            let facts = self
                .endpoints
                .get(id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                .ok_or("ENDPOINT_NOT_IN_PROJECT")?;
            if facts.summary.project_id != q.project_id {
                return Err("ENDPOINT_NOT_IN_PROJECT".into());
            }
            q.endpoint_id = Some(facts.summary.id);
        }
        if let Some(id) = q.endpoint_id {
            let s = summaries
                .iter()
                .find(|s| s.id == id)
                .ok_or("ENDPOINT_NOT_IN_PROJECT")?;
            if q.environment_id
                .is_some_and(|e| !s.environments.iter().any(|u| u.environment_id == e))
                && !s.environments.is_empty()
            {
                return Err("ENVIRONMENT_MISMATCH".into());
            }
        }
        let mut result = match tool {
            "browse_directory" => self.browse(&q, &summaries).await,
            "search_interfaces" => self.search(&q, &summaries).await,
            "read_interface" if q.section.is_none() => self.legacy_interface(&q).await,
            "read_interface" => self.read(&q, &summaries).await,
            "read_request_material" | "read_response_material" | "read_source_evidence" => {
                self.material(tool, &q, &summaries).await
            }
            "read_field_values" => self.values(&q).await,
            "trace_dependencies" => self.trace(&q, &summaries).await,
            "inspect_knowledge_coverage" => self.coverage(&q, &summaries).await,
            _ => Err("UNKNOWN_TOOL".into()),
        }?;
        lifecycle::envelope(&mut result, &q);
        Ok(result)
    }
}
