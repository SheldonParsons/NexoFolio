//! Discovery starts without a project ID and cannot read knowledge or raw samples.
use super::Navigation;
use nexofolio_access_contracts::McpPrincipal;
use nexofolio_common::{Error, ProjectId};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectsQuery {
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    limit: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentsQuery {
    project_id: ProjectId,
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    20
}
fn error_code(error: Error) -> &'static str {
    match error {
        Error::Unauthenticated => "UNAUTHENTICATED",
        Error::Forbidden => "PROJECT_ACCESS_DENIED",
        Error::InvalidInput { .. } => "INVALID_BOUNDS",
        _ => "ACCESS_UNAVAILABLE",
    }
}
impl Navigation {
    pub(crate) async fn discover(
        &self,
        principal: &McpPrincipal,
        tool: &str,
        args: Value,
    ) -> Result<Value, &'static str> {
        match tool {
            "list_projects" => {
                let q: ProjectsQuery =
                    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
                let result = self
                    .access
                    .list_mcp_projects(principal, q.offset, q.limit)
                    .await
                    .map_err(error_code)?;
                serde_json::to_value(result).map_err(|_| "ACCESS_UNAVAILABLE")
            }
            "list_project_environments" => {
                let q: EnvironmentsQuery =
                    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
                let result = self
                    .access
                    .list_mcp_environments(principal, q.project_id, q.offset, q.limit)
                    .await
                    .map_err(error_code)?;
                serde_json::to_value(result).map_err(|_| "ACCESS_UNAVAILABLE")
            }
            _ => Err("UNKNOWN_TOOL"),
        }
    }
}
