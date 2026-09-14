mod auth;
pub use auth::{McpRequestContext, require_token};

use rmcp::{
    ServerHandler,
    model::{Implementation, ServerCapabilities, ServerInfo},
};

#[derive(Clone)]
pub(crate) struct FoundationMcp;

impl ServerHandler for FoundationMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "nexofolio-foundation",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Foundation only. No knowledge tools are registered.")
    }
}
