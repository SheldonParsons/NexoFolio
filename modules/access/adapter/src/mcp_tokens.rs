//! Hash-only MCP credentials and current user/project authorization, owned by access.
use crate::{Postgres, session_crypto::SessionCrypto};
use async_trait::async_trait;
use nexofolio_access_contracts::{
    McpDiscoveryPage, McpEnvironment, McpPrincipal, McpProject, McpProjectAccess, McpTokenVerifier,
};
use nexofolio_common::{Error, ProjectId, Result, Secret, TokenId, UserId};
use sqlx::Row;
use uuid::Uuid;

fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "mcp_access",
    }
}
fn uuid(id: impl std::fmt::Display) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}

#[async_trait]
impl McpTokenVerifier for Postgres {
    async fn verify(&self, token: &Secret) -> Result<McpPrincipal> {
        let row = sqlx::query("SELECT t.id,t.user_id FROM mcp_tokens t JOIN project_access_principals u ON u.id=t.user_id WHERE t.token_hash=$1 AND t.revoked_at IS NULL AND t.starts_at<=now() AND (t.expires_at IS NULL OR t.expires_at>now()) AND u.ready")
            .bind(SessionCrypto::hash(token)).fetch_optional(&self.pool).await.map_err(db_error)?
            .ok_or(Error::Unauthenticated)?;
        Ok(McpPrincipal {
            user_id: row.get::<Uuid, _>("user_id").to_string().parse().unwrap(),
            token_id: row.get::<Uuid, _>("id").to_string().parse().unwrap(),
        })
    }
}

#[async_trait]
impl McpProjectAccess for Postgres {
    async fn list_mcp_projects(
        &self,
        principal: &McpPrincipal,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpProject>> {
        self.discover_mcp_projects(principal, offset, limit).await
    }
    async fn list_mcp_environments(
        &self,
        principal: &McpPrincipal,
        project: ProjectId,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpEnvironment>> {
        self.discover_mcp_environments(principal, project, offset, limit)
            .await
    }
    async fn require_mcp_project(
        &self,
        principal: &McpPrincipal,
        project: ProjectId,
    ) -> Result<()> {
        let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM effective_project_access a JOIN mcp_tokens t ON t.user_id=a.user_id WHERE a.user_id=$1 AND a.project_id=$2 AND t.id=$3 AND t.revoked_at IS NULL AND t.starts_at<=now() AND (t.expires_at IS NULL OR t.expires_at>now()))")
            .bind(uuid(principal.user_id)).bind(uuid(project)).bind(uuid(principal.token_id))
            .fetch_one(&self.pool).await.map_err(db_error)?;
        if allowed {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

impl Postgres {
    /// Administration only. Caller writes the secret to a protected file, never logs it.
    pub async fn issue_mcp_token(&self, user: UserId, days: u32) -> Result<(TokenId, Secret)> {
        if !(1..=30).contains(&days) {
            return Err(Error::invalid("days must be 1..30"));
        }
        let secret = SessionCrypto::random_mcp_token();
        let id = TokenId::new();
        let result = sqlx::query("INSERT INTO mcp_tokens(id,user_id,token_hash,expires_at) SELECT $1,id,$3,now()+($4*interval '1 day') FROM project_access_principals WHERE id=$2 AND ready")
            .bind(uuid(id)).bind(uuid(user)).bind(SessionCrypto::hash(&secret)).bind(i32::try_from(days).unwrap())
            .execute(&self.pool).await.map_err(db_error)?;
        if result.rows_affected() != 1 {
            return Err(Error::Forbidden);
        }
        Ok((id, secret))
    }
    pub async fn revoke_mcp_token(&self, id: TokenId) -> Result<()> {
        sqlx::query("UPDATE mcp_tokens SET revoked_at=now() WHERE id=$1")
            .bind(uuid(id))
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(())
    }
}
