use super::{PostgresAccess, db_error, profile, uuid};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use nexofolio_access_contracts::{
    PersonalProfile, PersonalProfiles, ProfileUpdate, SessionPrincipal,
};
use nexofolio_common::{Error, Result};
use sqlx::{Row, postgres::PgRow};
const PROFILE_SELECT: &str = "SELECT u.id,u.account,u.enabled,u.display_name AS account_display_name,coalesce(p.display_name,u.display_name) AS display_name,p.display_name AS display_name_override,coalesce(p.bio,'') AS bio,p.avatar_url,coalesce(p.version,0) AS version,u.created_at,u.last_login_at FROM users u LEFT JOIN profile_preferences p ON p.user_id=u.id WHERE u.id=$1 AND u.instance=$2 AND u.enabled";

impl PostgresAccess {
    async fn change_profile(
        &self,
        p: &SessionPrincipal,
        version: i64,
        text: Option<ProfileUpdate>,
        avatar: Option<Option<String>>,
    ) -> Result<PersonalProfile> {
        if version < 0 {
            return Err(Error::invalid("资料版本无效"));
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        let owned: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id=$1 AND instance=$2 AND enabled FOR UPDATE",
        )
        .bind(uuid(p.user_id))
        .bind(&p.instance)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        if owned.is_none() {
            return Err(Error::Unauthenticated);
        }
        sqlx::query("INSERT INTO profile_preferences(user_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(uuid(p.user_id))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let changed = if let Some(text) = text {
            sqlx::query("UPDATE profile_preferences SET display_name=$3,bio=$4,version=version+1,updated_at=clock_timestamp() WHERE user_id=$1 AND version=$2")
                .bind(uuid(p.user_id)).bind(version).bind(text.display_name).bind(text.bio).execute(&mut *tx).await.map_err(db_error)?
        } else {
            sqlx::query("UPDATE profile_preferences SET avatar_url=$3,version=version+1,updated_at=clock_timestamp() WHERE user_id=$1 AND version=$2")
                .bind(uuid(p.user_id)).bind(version).bind(avatar.flatten()).execute(&mut *tx).await.map_err(db_error)?
        };
        if changed.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        let row = sqlx::query(PROFILE_SELECT)
            .bind(uuid(p.user_id))
            .bind(&p.instance)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        let accepted = personal(&row)?;
        tx.commit().await.map_err(db_error)?;
        Ok(accepted)
    }
}
#[async_trait]
impl PersonalProfiles for PostgresAccess {
    async fn personal_profile(&self, p: &SessionPrincipal) -> Result<PersonalProfile> {
        let row = sqlx::query(PROFILE_SELECT)
            .bind(uuid(p.user_id))
            .bind(&p.instance)
            .fetch_optional(&self.database.pool)
            .await
            .map_err(db_error)?
            .ok_or(Error::Unauthenticated)?;
        personal(&row)
    }

    async fn update_profile(
        &self,
        p: &SessionPrincipal,
        update: ProfileUpdate,
    ) -> Result<PersonalProfile> {
        self.change_profile(p, update.version, Some(update), None)
            .await
    }
    async fn update_avatar(
        &self,
        p: &SessionPrincipal,
        version: i64,
        bytes: Option<Vec<u8>>,
    ) -> Result<PersonalProfile> {
        let image = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            match bytes {
                Some(bytes) => {
                    super::avatar::validate(&bytes)?;
                    Ok(Some(format!(
                        "data:image/webp;base64,{}",
                        STANDARD.encode(bytes)
                    )))
                }
                None => Ok(None),
            }
        })
        .await
        .map_err(|_| Error::Unavailable {
            component: "profile_image",
        })??;
        self.change_profile(p, version, None, Some(image)).await
    }
}

fn personal(row: &PgRow) -> Result<PersonalProfile> {
    Ok(PersonalProfile {
        user: profile(row)?,
        account_display_name: row.try_get("account_display_name").map_err(db_error)?,
        display_name_override: row.try_get("display_name_override").map_err(db_error)?,
        bio: row.try_get("bio").map_err(db_error)?,
        version: row.try_get("version").map_err(db_error)?,
        created_at: row
            .try_get::<DateTime<Utc>, _>("created_at")
            .map_err(db_error)?
            .to_rfc3339(),
        last_login_at: row
            .try_get::<DateTime<Utc>, _>("last_login_at")
            .map_err(db_error)?
            .to_rfc3339(),
    })
}
