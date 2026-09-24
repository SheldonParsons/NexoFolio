use async_trait::async_trait;
use nexofolio_common::{Result, Secret, TokenId, UserId};

#[derive(Debug, Clone)]
pub struct McpPrincipal {
    pub user_id: UserId,
    pub token_id: TokenId,
}

#[async_trait]
pub trait McpTokenVerifier: Send + Sync {
    /// Verify expiry, revocation, owner status and effective current grants.
    async fn verify(&self, token: &Secret) -> Result<McpPrincipal>;
}
