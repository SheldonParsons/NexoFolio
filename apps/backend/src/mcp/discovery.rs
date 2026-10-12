//! Discovery starts without a project ID; lifecycle metadata does not load document contents or raw samples.
use super::{Navigation, navigation::ReadError};
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
    ) -> Result<Value, ReadError> {
        match tool {
            "list_projects" => {
                let q: ProjectsQuery =
                    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
                if !(1..=50).contains(&q.limit) {
                    return Err(ReadError::bounds(
                        "limit",
                        q.limit as usize,
                        1,
                        50,
                        20,
                        "items",
                    ));
                }
                let result = self
                    .access
                    .list_mcp_projects(principal, q.offset, q.limit)
                    .await
                    .map_err(error_code)?;
                let mut value = serde_json::to_value(result)
                    .map_err(|_| ReadError::from("ACCESS_UNAVAILABLE"))?;
                if let Some(items) = value["items"].as_array_mut() {
                    for item in items {
                        let id = serde_json::from_value(item["project_id"].clone())
                            .map_err(|_| "ACCESS_UNAVAILABLE")?;
                        let lifecycle = self
                            .knowledge
                            .lifecycle(id)
                            .await
                            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                        if !lifecycle.is_empty() {
                            item["lifecycle"] = super::navigation::lifecycle::summary(&lifecycle);
                        }
                    }
                }
                Ok(value)
            }
            "list_project_environments" => {
                let q: EnvironmentsQuery =
                    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS")?;
                if !(1..=50).contains(&q.limit) {
                    return Err(ReadError::bounds(
                        "limit",
                        q.limit as usize,
                        1,
                        50,
                        20,
                        "items",
                    ));
                }
                let result = self
                    .access
                    .list_mcp_environments(principal, q.project_id, q.offset, q.limit)
                    .await
                    .map_err(error_code)?;
                let mut value = serde_json::to_value(result)
                    .map_err(|_| ReadError::from("ACCESS_UNAVAILABLE"))?;
                let lifecycle = self
                    .knowledge
                    .lifecycle(q.project_id)
                    .await
                    .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
                if !lifecycle.is_empty() {
                    value["project_lifecycle"] = super::navigation::lifecycle::summary(&lifecycle);
                }
                Ok(value)
            }
            _ => Err("UNKNOWN_TOOL".into()),
        }
    }
}
