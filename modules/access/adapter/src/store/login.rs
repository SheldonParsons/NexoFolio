use super::{PostgresAccess, db_error, profile, uuid};
use crate::session_crypto::SessionCrypto;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_access_contracts::{
    ExternalIdentity, LoginStore, ProjectSnapshot, SessionLogin, SessionPrincipal, SyncCounts,
    UserProfile,
};
use nexofolio_common::{Error, Result};
use sqlx::{Postgres as Pg, Row, Transaction};
use uuid::Uuid;

impl PostgresAccess {
    async fn session(
        &self,
        tx: &mut Transaction<'_, Pg>,
        user: UserProfile,
        method: &str,
    ) -> Result<SessionLogin> {
        if !user.enabled {
            return Err(Error::Forbidden);
        }
        // The caller holds the user row lock; concurrent logins cannot rotate a live token.
        let existing=sqlx::query("SELECT token_encrypted,expires_at FROM internal_sessions WHERE user_id=$1 AND expires_at>clock_timestamp() AND revoked_at IS NULL")
        .bind(uuid(user.id)).fetch_optional(&mut **tx).await.map_err(db_error)?;
        let (token, expires, reused) = if let Some(row) = existing {
            (
                self.crypto.decrypt(
                    &row.try_get::<Vec<u8>, _>("token_encrypted")
                        .map_err(db_error)?,
                    &user.id.to_string(),
                )?,
                row.try_get::<DateTime<Utc>, _>("expires_at")
                    .map_err(db_error)?,
                true,
            )
        } else {
            let token = self.crypto.generate();
            let ciphertext = self.crypto.encrypt(&token, &user.id.to_string())?;
            let row=sqlx::query(r#"INSERT INTO internal_sessions(user_id,token_hash,token_encrypted,issued_at,expires_at)
            VALUES($1,$2,$3,clock_timestamp(),(clock_timestamp() AT TIME ZONE 'UTC' + interval '3 months') AT TIME ZONE 'UTC')
            ON CONFLICT(user_id) DO UPDATE
            SET token_hash=excluded.token_hash,token_encrypted=excluded.token_encrypted,issued_at=excluded.issued_at,expires_at=excluded.expires_at,revoked_at=NULL
            RETURNING expires_at"#)
            .bind(uuid(user.id)).bind(SessionCrypto::hash(&token)).bind(ciphertext).fetch_one(&mut **tx).await.map_err(db_error)?;
            (
                token,
                row.try_get::<DateTime<Utc>, _>("expires_at")
                    .map_err(db_error)?,
                false,
            )
        };
        sqlx::query("INSERT INTO login_audit(user_id,method) VALUES($1,$2)")
            .bind(uuid(user.id))
            .bind(method)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
        Ok(SessionLogin {
            user,
            token,
            expires_at: expires.to_rfc3339(),
            reused,
        })
    }
}

#[async_trait]
impl LoginStore for PostgresAccess {
    async fn normal_login(&self, i: &ExternalIdentity) -> Result<SessionLogin> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let row=sqlx::query(r#"INSERT INTO users(id,instance,external_id,account,display_name)
            VALUES($1,$2,$3,$4,$5)
            ON CONFLICT(instance,external_id) DO UPDATE
            SET account=excluded.account,display_name=excluded.display_name,last_login_at=clock_timestamp()
            RETURNING id,account,display_name,enabled"#)
        .bind(Uuid::new_v4()).bind(&i.instance).bind(&i.external_id).bind(&i.account).bind(&i.display_name).fetch_one(&mut *tx).await.map_err(db_error)?;
        let result = self.session(&mut tx, profile(&row)?, "zentao").await?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn emergency_login(&self, instance: &str, account: &str) -> Result<SessionLogin> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let row=sqlx::query("SELECT id,account,display_name,enabled FROM users WHERE instance=$1 AND lower(account)=lower($2) FOR UPDATE")
        .bind(instance).bind(account).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or(Error::Unauthenticated)?;
        let result = self.session(&mut tx, profile(&row)?, "emergency").await?;
        sqlx::query("UPDATE users SET last_login_at=clock_timestamp() WHERE id=$1")
            .bind(uuid(result.user.id))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn next_sync_generation(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT nextval('access_sync_generation')")
            .fetch_one(&self.database.pool)
            .await
            .map_err(db_error)
    }
    async fn apply_snapshot(
        &self,
        p: &SessionPrincipal,
        generation: i64,
        mut snapshot: ProjectSnapshot,
    ) -> Result<SyncCounts> {
        if snapshot.projects.iter().any(|x| x.instance != p.instance) {
            return Err(Error::Forbidden);
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let current:Option<i64>=sqlx::query_scalar("SELECT last_sync_generation FROM users WHERE id=$1 AND instance=$2 AND enabled FOR UPDATE")
        .bind(uuid(p.user_id)).bind(&p.instance).fetch_optional(&mut *tx).await.map_err(db_error)?;
        let current = current.ok_or(Error::Unauthenticated)?;
        if current >= generation {
            return Err(Error::Conflict);
        }
        snapshot
            .projects
            .sort_by(|a, b| a.external_id.cmp(&b.external_id));
        let mut counts = SyncCounts::default();
        for project in &snapshot.projects {
            let created = if project.state == "doing" {
                sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(instance,external_id) DO NOTHING")
                .bind(Uuid::new_v4()).bind(&p.instance).bind(&project.external_id).bind(&project.name).bind(&project.state).bind(generation).execute(&mut *tx).await.map_err(db_error)?.rows_affected()
            } else {
                0
            };
            if created == 1 {
                counts.created += 1;
            } else {
                let n=sqlx::query("UPDATE projects SET status=$1,last_sync_generation=$2,last_synced_at=clock_timestamp() WHERE instance=$3 AND external_id=$4 AND last_sync_generation<$2")
                .bind(&project.state).bind(generation).bind(&p.instance).bind(&project.external_id).execute(&mut *tx).await.map_err(db_error)?.rows_affected();
                if n > 0 {
                    counts.updated += 1;
                } else {
                    counts.skipped += 1;
                }
            }
        }
        sqlx::query("DELETE FROM user_project_access WHERE user_id=$1")
            .bind(uuid(p.user_id))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        sqlx::query("INSERT INTO user_project_access(user_id,project_id) SELECT $1,id FROM projects WHERE instance=$2 AND external_id=ANY($3)")
        .bind(uuid(p.user_id)).bind(&p.instance).bind(&snapshot.visible_project_ids).execute(&mut *tx).await.map_err(db_error)?;
        sqlx::query("UPDATE users SET grants_synced=true,last_sync_generation=$2 WHERE id=$1")
            .bind(uuid(p.user_id))
            .bind(generation)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(counts)
    }
}
