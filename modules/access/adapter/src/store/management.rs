use super::{PostgresAccess, db_error, typed, uuid};
use nexofolio_access_contracts::{PermissionSource, PermissionSources, SessionPrincipal};
use nexofolio_common::{Error, Result};
use sqlx::{PgConnection, Row};

pub(super) async fn require_manager(
    connection: &mut PgConnection,
    principal: &SessionPrincipal,
) -> Result<()> {
    let exists: Option<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 AND instance=$2 AND enabled AND lower(account)='a80646' FOR SHARE")
        .bind(uuid(principal.user_id)).bind(&principal.instance).fetch_optional(connection).await.map_err(db_error)?;
    exists.map(|_| ()).ok_or(Error::NotFound)
}

impl PostgresAccess {
    pub async fn permission_sources(
        &self,
        principal: &SessionPrincipal,
        page: u32,
        limit: u32,
    ) -> Result<PermissionSources> {
        if page == 0 || page > 100000 || !(1..=100).contains(&limit) {
            return Err(Error::invalid("invalid pagination"));
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM projects WHERE instance=$1 AND origin='zentao'",
        )
        .bind(&principal.instance)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        let rows = sqlx::query("SELECT id,name,status FROM projects WHERE instance=$1 AND origin='zentao' ORDER BY name,id LIMIT $2 OFFSET $3")
            .bind(&principal.instance).bind(i64::from(limit)).bind(i64::from((page-1)*limit)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .into_iter()
            .map(|r| PermissionSource {
                project_id: typed(r.get("id")),
                name: r.get("name"),
                status: r.get("status"),
            })
            .collect();
        tx.commit().await.map_err(db_error)?;
        Ok(PermissionSources {
            items,
            total: total as u64,
            page,
            limit,
        })
    }
}
