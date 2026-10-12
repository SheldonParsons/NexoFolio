use super::{PostgresAccess, db_error, management::require_manager, typed};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_access_contracts::{
    ManagementUser, ManagementUsers, SessionPrincipal, UserDirectory, validate_user_directory_query,
};
use nexofolio_common::Result;
use sqlx::Row;

#[async_trait]
impl UserDirectory for PostgresAccess {
    async fn list_registered_users(
        &self,
        principal: &SessionPrincipal,
        page: u32,
        limit: u32,
        query: &str,
    ) -> Result<ManagementUsers> {
        validate_user_directory_query(page, limit, query)?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        // Match literal text, including '%' and '_'; never search another instance.
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users u LEFT JOIN profile_preferences p ON p.user_id=u.id \
             WHERE u.instance=$1 AND (strpos(lower(u.account),lower($2))>0 \
             OR strpos(lower(coalesce(p.display_name,u.display_name)),lower($2))>0)",
        )
        .bind(&principal.instance)
        .bind(query.trim())
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        let rows = sqlx::query(
            "SELECT u.id,u.account,coalesce(p.display_name,u.display_name) AS display_name,\
             p.avatar_url,u.enabled,u.created_at,u.last_login_at \
             FROM users u LEFT JOIN profile_preferences p ON p.user_id=u.id \
             WHERE u.instance=$1 AND (strpos(lower(u.account),lower($2))>0 \
             OR strpos(lower(coalesce(p.display_name,u.display_name)),lower($2))>0) \
             ORDER BY u.created_at DESC,u.id DESC LIMIT $3 OFFSET $4",
        )
        .bind(&principal.instance)
        .bind(query.trim())
        .bind(i64::from(limit))
        .bind(i64::from(page - 1) * i64::from(limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let items = rows
            .into_iter()
            .map(|row| ManagementUser {
                id: typed(row.get("id")),
                account: row.get("account"),
                display_name: row.get("display_name"),
                avatar_url: row.get("avatar_url"),
                enabled: row.get("enabled"),
                created_at: row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
                last_login_at: row.get::<DateTime<Utc>, _>("last_login_at").to_rfc3339(),
            })
            .collect();
        tx.commit().await.map_err(db_error)?;
        Ok(ManagementUsers {
            items,
            total: total as u64,
            page,
            limit,
        })
    }
}
