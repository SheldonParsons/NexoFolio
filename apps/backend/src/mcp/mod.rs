mod auth;
mod discovery;
mod navigation;
#[cfg(test)]
mod request_tests;
mod tools;
pub use auth::{McpRequestContext, require_token};
pub use navigation::Navigation;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
};
#[derive(Clone, Default)]
pub struct KnowledgeMcp {
    navigation: Option<Navigation>,
}
impl KnowledgeMcp {
    pub fn new(navigation: Option<Navigation>) -> Self {
        Self { navigation }
    }
}
impl ServerHandler for KnowledgeMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                if self.navigation.is_some() { "nexofolio-knowledge" } else { "nexofolio-foundation" }, env!("CARGO_PKG_VERSION")))
            .with_instructions("Start with list_projects (no project_id required), then list_project_environments for the chosen project. Follow next_offset while has_more=true. Use returned project_id and environment_id to search, read and trace saved scoped knowledge; ask the user to choose when names are ambiguous. Do not guess IDs or mix environments. Observed inputs/relations are not backend requirements. Supply current inputs and business authentication state. Tools do not execute business APIs.")
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: if self.navigation.is_some() {
                tools::definitions()
            } else {
                vec![]
            },
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let principal = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<McpRequestContext>())
            .map(|ctx| &ctx.principal);
        let (Some(navigation), Some(principal)) = (&self.navigation, principal) else {
            return Ok(CallToolResult::structured_error(
                serde_json::json!({"error":"UNAUTHENTICATED"}),
            )
            .into());
        };
        match navigation
            .call(
                principal,
                &request.name,
                serde_json::Value::Object(request.arguments.unwrap_or_default()),
            )
            .await
        {
            Ok(value) => {
                if value.to_string().len() > 128 * 1024 {
                    return Ok(CallToolResult::structured_error(serde_json::json!({"error":"OUTPUT_TOO_LARGE","suggestion":"Reduce limit/max_steps and continue paging"})).into());
                }
                Ok(CallToolResult::structured(value).into())
            }
            Err(error) => {
                Ok(CallToolResult::structured_error(serde_json::json!({"error":error})).into())
            }
        }
    }
}
