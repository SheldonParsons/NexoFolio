//! Read-only MCP discovery from the same live grants used by knowledge tools.
use crate::Postgres;
use nexofolio_access_contracts::{McpDiscoveryPage, McpEnvironment, McpPrincipal, McpProject};
use nexofolio_common::{Error, ProjectId, Result};
use sqlx::{Postgres as Pg, Row, Transaction};
use uuid::Uuid;

fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "mcp_access",
    }
}
fn uuid(id: impl std::fmt::Display) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}
fn bounds(offset: u64, limit: u32) -> Result<i64> {
    if !(1..=50).contains(&limit) || offset > (i64::MAX as u64) - 50 {
        return Err(Error::invalid("invalid discovery pagination"));
    }
    Ok(offset as i64)
}
fn page<T>(items: Vec<T>, total: i64, offset: u64, limit: u32) -> McpDiscoveryPage<T> {
    let next = offset + items.len() as u64;
    let has_more = next < total as u64;
    McpDiscoveryPage {
        items,
        total: total as u64,
        offset,
        limit,
        has_more,
        next_offset: has_more.then_some(next),
    }
}

async fn owner(tx: &mut Transaction<'_, Pg>, principal: &McpPrincipal) -> Result<String> {
    sqlx::query_scalar("SELECT u.instance FROM mcp_tokens t JOIN users u ON u.id=t.user_id WHERE t.id=$1 AND u.id=$2 AND t.revoked_at IS NULL AND t.starts_at<=now() AND (t.expires_at IS NULL OR t.expires_at>now()) AND u.enabled AND u.grants_synced")
        .bind(uuid(principal.token_id)).bind(uuid(principal.user_id))
        .fetch_optional(&mut **tx).await.map_err(db_error)?.ok_or(Error::Unauthenticated)
}

impl Postgres {
    pub(crate) async fn discover_mcp_projects(
        &self,
        principal: &McpPrincipal,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpProject>> {
        let skip = bounds(offset, limit)?;
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let instance = owner(&mut tx, principal).await?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM projects p JOIN user_project_access a ON a.project_id=p.id WHERE a.user_id=$1 AND p.instance=$2")
            .bind(uuid(principal.user_id)).bind(&instance).fetch_one(&mut *tx).await.map_err(db_error)?;
        let rows = sqlx::query("SELECT p.id,p.name,p.status FROM projects p JOIN user_project_access a ON a.project_id=p.id WHERE a.user_id=$1 AND p.instance=$2 ORDER BY p.id LIMIT $3 OFFSET $4")
            .bind(uuid(principal.user_id)).bind(&instance).bind(i64::from(limit)).bind(skip)
            .fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .iter()
            .map(|row| {
                Ok(McpProject {
                    project_id: row
                        .try_get::<Uuid, _>("id")
                        .map_err(db_error)?
                        .to_string()
                        .parse()
                        .expect("UUID"),
                    name: row.try_get("name").map_err(db_error)?,
                    status: row.try_get("status").map_err(db_error)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(page(items, total, offset, limit))
    }

    pub(crate) async fn discover_mcp_environments(
        &self,
        principal: &McpPrincipal,
        project: ProjectId,
        offset: u64,
        limit: u32,
    ) -> Result<McpDiscoveryPage<McpEnvironment>> {
        let skip = bounds(offset, limit)?;
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let instance = owner(&mut tx, principal).await?;
        let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects p JOIN user_project_access a ON a.project_id=p.id WHERE p.id=$1 AND a.user_id=$2 AND p.instance=$3)")
            .bind(uuid(project)).bind(uuid(principal.user_id)).bind(&instance)
            .fetch_one(&mut *tx).await.map_err(db_error)?;
        if !allowed {
            return Err(Error::Forbidden);
        }
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM environments WHERE project_id=$1")
                .bind(uuid(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let rows = sqlx::query(
            "SELECT id,name FROM environments WHERE project_id=$1 ORDER BY id LIMIT $2 OFFSET $3",
        )
        .bind(uuid(project))
        .bind(i64::from(limit))
        .bind(skip)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let items = rows
            .iter()
            .map(|row| {
                Ok(McpEnvironment {
                    project_id: project,
                    environment_id: row
                        .try_get::<Uuid, _>("id")
                        .map_err(db_error)?
                        .to_string()
                        .parse()
                        .expect("UUID"),
                    name: row.try_get("name").map_err(db_error)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(page(items, total, offset, limit))
    }
}
