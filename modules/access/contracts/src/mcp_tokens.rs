use crate::SessionPrincipal;
use async_trait::async_trait;
use nexofolio_common::{EnvironmentId, ProjectId, Result, Secret, TokenId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateMcpToken {
    pub name: String,
    /// RFC 3339; omitted/null means immediately.
    pub starts_at: Option<String>,
    /// RFC 3339; omitted/null means no expiry.
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTokenInfo {
    pub id: TokenId,
    pub name: String,
    pub issued_at: String,
    pub starts_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub status: McpTokenStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTokenStatus {
    Scheduled,
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Serialize)]
pub struct McpTokenPage {
    pub items: Vec<McpTokenInfo>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
}

/// The secret is deliberately not serializable; only creation HTTP may expose it.
#[derive(Debug)]
pub struct CreatedMcpToken {
    pub credential: McpTokenInfo,
    pub token: Secret,
}

/// Self-service operations use the authenticated web principal, never a supplied owner.
#[async_trait]
pub trait McpTokens: Send + Sync {
    /// None means the account has no established permission snapshot yet.
    async fn mcp_accessible_projects(&self, owner: &SessionPrincipal) -> Result<Option<i64>>;
    async fn list_mcp_tokens(
        &self,
        owner: &SessionPrincipal,
        page: u32,
        limit: u32,
    ) -> Result<McpTokenPage>;
    async fn create_mcp_token(
        &self,
        owner: &SessionPrincipal,
        input: CreateMcpToken,
    ) -> Result<CreatedMcpToken>;
    async fn revoke_owned_mcp_token(&self, owner: &SessionPrincipal, id: TokenId) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct McpPrincipal {
    pub user_id: UserId,
    pub token_id: TokenId,
}

#[derive(Debug, Serialize)]
pub struct McpProject {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct McpEnvironment {
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct McpDiscoveryPage<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u32,
    pub has_more: bool,
    pub next_offset: Option<u64>,
}

#[async_trait]
pub trait McpTokenVerifier: Send + Sync {
    /// Verify expiry, revocation, owner status and effective current grants.
    async fn verify(&self, token: &Secret) -> Result<McpPrincipal>;
}

/// Check effective project grants for the current MCP owner on every tool call.
#[async_trait]
pub trait McpProjectAccess: Send + Sync {
    /// Only currently authorized projects; a project ID is not needed to start discovery.
    async fn list_mcp_projects(
        &self,
        principal: &McpPrincipal,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpProject>>;
    async fn list_mcp_environments(
        &self,
        principal: &McpPrincipal,
        project: ProjectId,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpEnvironment>>;
    async fn require_mcp_project(
        &self,
        principal: &McpPrincipal,
        project: nexofolio_common::ProjectId,
    ) -> Result<()>;
}
