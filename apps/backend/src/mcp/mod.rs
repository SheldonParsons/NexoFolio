mod auth;
mod discovery;
#[cfg(test)]
mod exploration_tests;
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
            .with_instructions(if self.navigation.is_some() {
                include_str!("instructions.md")
            } else {
                "当前服务尚未装配知识导航工具。空工具列表不代表项目没有接口资料；请联系部署方确认服务配置。"
            })
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
        let started = std::time::Instant::now();
        let audit = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<crate::audit::Context>())
            .cloned();
        static AUDIT_TOOL_NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let names = AUDIT_TOOL_NAMES.get_or_init(|| {
            tools::definitions()
                .iter()
                .map(|t| t.name.to_string())
                .collect()
        });
        if let Some(audit) = &audit {
            let id = |key: &str| {
                request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get(key))
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
            };
            audit.scope(
                id("project_id"),
                id("endpoint_id").or_else(|| id("directory_id")),
            );
        }
        let tool = if names.iter().any(|name| name == &request.name) {
            request.name.to_string()
        } else {
            "unknown_tool".to_string()
        };
        let record = |code: &str| {
            if let Some(audit) = &audit {
                audit.record(
                    "mcp",
                    &tool,
                    code,
                    None,
                    started.elapsed().as_millis().min(u32::MAX as u128) as u32,
                );
            }
        };
        let principal = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<McpRequestContext>())
            .map(|ctx| &ctx.principal);
        let (Some(navigation), Some(principal)) = (&self.navigation, principal) else {
            record("UNAUTHENTICATED");
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
                    record("OUTPUT_TOO_LARGE");
                    return Ok(CallToolResult::structured_error(serde_json::json!({"error":"OUTPUT_TOO_LARGE","suggestion":"Reduce limit/max_steps and continue paging"})).into());
                }
                record("OK");
                Ok(CallToolResult::structured(value).into())
            }
            Err(error) => {
                record(error.code());
                Ok(CallToolResult::structured_error(error.payload()).into())
            }
        }
    }
}
