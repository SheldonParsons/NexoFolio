use super::{PostgresAccess, db_error, typed, uuid};
use async_trait::async_trait;
use nexofolio_access_contracts::{ActorName, ActorNames};
use nexofolio_common::{Result, UserId};
use sqlx::Row;
#[async_trait]
impl ActorNames for PostgresAccess {
    async fn actor_names(&self, ids: &[UserId]) -> Result<Vec<ActorName>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<_> = ids.iter().copied().map(uuid).collect();
        // Disabled actors remain identifiable in history. No account, credential or avatar payload.
        let rows = sqlx::query("SELECT u.id,coalesce(p.display_name,u.display_name) AS display_name FROM users u LEFT JOIN profile_preferences p ON p.user_id=u.id WHERE u.id=ANY($1)")
            .bind(ids).fetch_all(&self.database.pool).await.map_err(db_error)?;
        Ok(rows
            .iter()
            .map(|r| ActorName {
                id: typed(r.get("id")),
                display_name: r.get("display_name"),
            })
            .collect())
    }
}
