use async_trait::async_trait;
use nexofolio_access_contracts::{McpPrincipal, McpTokenVerifier};
use nexofolio_common::{Error, Result, Secret};

/// Production default until MCP tokens exist. Never authenticates anyone.
#[derive(Debug, Default)]
pub struct Unconfigured;

#[async_trait]
impl McpTokenVerifier for Unconfigured {
    async fn verify(&self, _: &Secret) -> Result<McpPrincipal> {
        Err(Error::NotConfigured {
            capability: "mcp_token_verification",
        })
    }
}
