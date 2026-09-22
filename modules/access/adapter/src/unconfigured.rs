use async_trait::async_trait;
use nexofolio_access_contracts::*;
use nexofolio_common::*;

/// Production default. Never authenticates, invents projects, returns fake records or runs jobs.
#[derive(Debug, Default)]
pub struct Unconfigured;

fn unavailable<T>(capability: &'static str) -> Result<T> {
    Err(Error::NotConfigured { capability })
}

#[async_trait]
impl ExternalAuthenticator for Unconfigured {
    async fn authenticate(&self, _: &LoginCredentials) -> Result<ExternalIdentity> {
        unavailable("zentao_auth")
    }
}
#[async_trait]
impl EmergencyAuthenticator for Unconfigured {
    async fn authenticate_existing(&self, _: &LoginCredentials) -> Result<UserProfile> {
        unavailable("emergency_auth")
    }
}
#[async_trait]
impl UserDirectory for Unconfigured {
    async fn find_existing(&self, _: &str) -> Result<Option<UserProfile>> {
        unavailable("users")
    }
}
#[async_trait]
impl ProjectSource for Unconfigured {
    async fn list_projects(&self, _: Option<&str>) -> Result<ProjectSourcePage> {
        unavailable("zentao_projects")
    }
}
#[async_trait]
impl ProjectPermissionSource for Unconfigured {
    async fn grants_for(&self, _: UserId) -> Result<Vec<ProjectGrant>> {
        unavailable("zentao_permissions")
    }
}
#[async_trait]
impl McpTokenVerifier for Unconfigured {
    async fn verify(&self, _: &Secret) -> Result<McpPrincipal> {
        unavailable("mcp_token_verification")
    }
}
#[async_trait]
impl McpTokenManager for Unconfigured {
    async fn issue(&self, _: IssueMcpToken) -> Result<IssuedMcpToken> {
        unavailable("mcp_token_issuance")
    }
    async fn revoke(&self, _: UserId, _: TokenId) -> Result<()> {
        unavailable("mcp_token_revocation")
    }
}
