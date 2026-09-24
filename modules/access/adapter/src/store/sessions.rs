use super::{PostgresAccess, db_error, profile, typed, uuid};
use crate::session_crypto::SessionCrypto;
use async_trait::async_trait;
use nexofolio_access_contracts::{SessionPrincipal, Sessions, UserProfile};
use nexofolio_common::{Error, Result, Secret};
use sqlx::Row;

#[async_trait]
impl Sessions for PostgresAccess {
    async fn verify_session(&self, token: &Secret) -> Result<SessionPrincipal> {
        if !token.expose().starts_with("nfi_") || token.expose().len() != 68 {
            return Err(Error::Unauthenticated);
        }
        let row=sqlx::query("SELECT u.id,u.instance FROM internal_sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>clock_timestamp() AND s.revoked_at IS NULL AND u.enabled")
        .bind(SessionCrypto::hash(token)).fetch_optional(&self.database.pool).await.map_err(db_error)?.ok_or(Error::Unauthenticated)?;
        Ok(SessionPrincipal {
            user_id: typed(row.try_get("id").map_err(db_error)?),
            instance: row.try_get("instance").map_err(db_error)?,
        })
    }
    async fn me(&self, p: &SessionPrincipal) -> Result<UserProfile> {
        let row=sqlx::query("SELECT id,account,display_name,enabled FROM users WHERE id=$1 AND instance=$2 AND enabled").bind(uuid(p.user_id)).bind(&p.instance).fetch_optional(&self.database.pool).await.map_err(db_error)?.ok_or(Error::Unauthenticated)?;
        profile(&row)
    }
}
