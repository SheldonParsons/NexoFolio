use crate::ProjectGrant;
use async_trait::async_trait;
use nexofolio_contracts::{ProjectId, Result, Secret, TokenId, UserId};

#[derive(Debug, Clone)]
pub struct McpPrincipal {
    pub user_id: UserId,
    pub token_id: TokenId,
    pub grants: Vec<ProjectGrant>,
}

#[derive(Debug, Clone, Copy)]
pub enum McpScope {
    KnowledgeRead,
    KnowledgeWrite,
}

#[derive(Debug)]
pub struct IssueMcpToken {
    pub owner: UserId,
    pub name: String,
    pub projects: Vec<ProjectId>,
    pub scopes: Vec<McpScope>,
    pub expires_at_unix: u64,
}

#[derive(Debug)]
pub struct IssuedMcpToken {
    pub id: TokenId,
    pub secret: Secret,
}

#[async_trait]
pub trait McpTokenVerifier: Send + Sync {
    /// Verify expiry, revocation, owner status and effective current grants.
    async fn verify(&self, token: &Secret) -> Result<McpPrincipal>;
}

#[async_trait]
pub trait McpTokenManager: Send + Sync {
    async fn issue(&self, request: IssueMcpToken) -> Result<IssuedMcpToken>;
    async fn revoke(&self, actor: UserId, token_id: TokenId) -> Result<()>;
}
