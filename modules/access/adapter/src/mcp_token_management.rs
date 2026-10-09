//! Owner-scoped credential management. No project grants are copied into a token.
use crate::{Postgres, session_crypto::SessionCrypto};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc};
use nexofolio_access_contracts::{
    CreateMcpToken, CreatedMcpToken, McpTokenInfo, McpTokenPage, McpTokenStatus, McpTokens,
    SessionPrincipal,
};
use nexofolio_common::{Error, Result, TokenId};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "mcp_access",
    }
}
fn uuid(id: impl std::fmt::Display) -> Uuid {
    id.to_string().parse().expect("typed UUID")
}

fn parse_time(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|s| {
            let date = DateTime::parse_from_rfc3339(s)
                .map_err(|_| Error::invalid("时间须包含时区，使用 RFC 3339 格式"))?
                .with_timezone(&Utc);
            if !(1..=9999).contains(&date.year()) {
                return Err(Error::invalid("时间超出支持范围"));
            }
            Ok(date)
        })
        .transpose()
}

fn validate(
    input: &CreateMcpToken,
    now: DateTime<Utc>,
) -> Result<(String, DateTime<Utc>, Option<DateTime<Utc>>)> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(Error::invalid("凭证名称须为 1～64 字，不能包含控制字符"));
    }
    let start = parse_time(input.starts_at.as_deref())?.unwrap_or(now);
    let end = parse_time(input.expires_at.as_deref())?;
    if end.is_some_and(|end| end <= start || end <= now) {
        return Err(Error::invalid("结束时间须晚于开始时间和当前时间"));
    }
    Ok((name.to_owned(), start, end))
}

fn info(row: &PgRow, now: DateTime<Utc>) -> Result<McpTokenInfo> {
    let start: DateTime<Utc> = row.try_get("starts_at").map_err(db_error)?;
    let end: Option<DateTime<Utc>> = row.try_get("expires_at").map_err(db_error)?;
    let revoked: Option<DateTime<Utc>> = row.try_get("revoked_at").map_err(db_error)?;
    let status = if revoked.is_some() {
        McpTokenStatus::Revoked
    } else if end.is_some_and(|end| end <= now) {
        McpTokenStatus::Expired
    } else if start > now {
        McpTokenStatus::Scheduled
    } else {
        McpTokenStatus::Active
    };
    Ok(McpTokenInfo {
        id: row
            .try_get::<Uuid, _>("id")
            .map_err(db_error)?
            .to_string()
            .parse()
            .expect("UUID"),
        name: row.try_get("name").map_err(db_error)?,
        issued_at: row
            .try_get::<DateTime<Utc>, _>("issued_at")
            .map_err(db_error)?
            .to_rfc3339(),
        starts_at: start.to_rfc3339(),
        expires_at: end.map(|d| d.to_rfc3339()),
        revoked_at: revoked.map(|d| d.to_rfc3339()),
        status,
    })
}

#[async_trait]
impl McpTokens for Postgres {
    async fn mcp_accessible_projects(&self, owner: &SessionPrincipal) -> Result<Option<i64>> {
        let row = sqlx::query("SELECT CASE WHEN u.grants_synced THEN (SELECT count(*) FROM user_project_access a JOIN projects p ON p.id=a.project_id AND p.instance=u.instance WHERE a.user_id=u.id) ELSE NULL END AS accessible_projects FROM users u WHERE u.id=$1 AND u.instance=$2 AND u.enabled")
            .bind(uuid(owner.user_id)).bind(&owner.instance)
            .fetch_optional(&self.pool).await.map_err(db_error)?
            .ok_or(Error::Unauthenticated)?;
        row.try_get("accessible_projects").map_err(db_error)
    }

    async fn list_mcp_tokens(
        &self,
        owner: &SessionPrincipal,
        page: u32,
        limit: u32,
    ) -> Result<McpTokenPage> {
        if !(1..=100_000).contains(&page) || !(1..=100).contains(&limit) {
            return Err(Error::invalid("分页参数超出范围"));
        }
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        // A consistent page and count, including when another client creates/revokes.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let now: DateTime<Utc> =
            sqlx::query_scalar("SELECT now() FROM users WHERE id=$1 AND instance=$2 AND enabled")
                .bind(uuid(owner.user_id))
                .bind(&owner.instance)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?
                .ok_or(Error::Unauthenticated)?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM mcp_tokens WHERE user_id=$1")
            .bind(uuid(owner.user_id))
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        let rows = sqlx::query("SELECT id,name,issued_at,starts_at,expires_at,revoked_at FROM mcp_tokens WHERE user_id=$1 ORDER BY issued_at DESC,id DESC LIMIT $2 OFFSET $3")
            .bind(uuid(owner.user_id)).bind(i64::from(limit)).bind(i64::from(page - 1) * i64::from(limit))
            .fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows
            .iter()
            .map(|r| info(r, now))
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(McpTokenPage {
            items,
            total,
            page,
            limit,
        })
    }

    async fn create_mcp_token(
        &self,
        owner: &SessionPrincipal,
        input: CreateMcpToken,
    ) -> Result<CreatedMcpToken> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        let now: DateTime<Utc> = sqlx::query_scalar(
            "SELECT now() FROM users WHERE id=$1 AND instance=$2 AND enabled FOR SHARE",
        )
        .bind(uuid(owner.user_id))
        .bind(&owner.instance)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or(Error::Unauthenticated)?;
        let (name, start, end) = validate(&input, now)?;
        let token = SessionCrypto::random_mcp_token();
        let id = TokenId::new();
        let row = sqlx::query("INSERT INTO mcp_tokens(id,user_id,token_hash,name,starts_at,expires_at) VALUES($1,$2,$3,$4,$5,$6) RETURNING id,name,issued_at,starts_at,expires_at,revoked_at")
            .bind(uuid(id)).bind(uuid(owner.user_id)).bind(SessionCrypto::hash(&token))
            .bind(name).bind(start).bind(end).fetch_one(&mut *tx).await.map_err(db_error)?;
        let credential = info(&row, now)?;
        tx.commit().await.map_err(db_error)?;
        Ok(CreatedMcpToken { credential, token })
    }

    async fn revoke_owned_mcp_token(&self, owner: &SessionPrincipal, id: TokenId) -> Result<()> {
        let result = sqlx::query("UPDATE mcp_tokens t SET revoked_at=coalesce(t.revoked_at,now()) FROM users u WHERE t.id=$1 AND t.user_id=$2 AND u.id=t.user_id AND u.instance=$3 AND u.enabled")
            .bind(uuid(id)).bind(uuid(owner.user_id)).bind(&owner.instance)
            .execute(&self.pool).await.map_err(db_error)?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_validity_windows_are_validated_without_losing_timezone() {
        let now = parse_time(Some("2026-10-09T00:00:00Z")).unwrap().unwrap();
        let mut input = CreateMcpToken {
            name: " 我的客户端 ".into(),
            starts_at: None,
            expires_at: None,
        };
        let (name, start, end) = validate(&input, now).unwrap();
        assert_eq!(name, "我的客户端");
        assert_eq!(start, now);
        assert!(end.is_none());
        input.starts_at = Some("2026-10-10T08:00:00+08:00".into());
        input.expires_at = Some("2026-10-10T00:00:00Z".into());
        assert!(validate(&input, now).is_err());
        input.expires_at = Some("2026-10-11T00:00:00Z".into());
        assert!(validate(&input, now).is_ok());
        input.starts_at = Some("2026-10-10T08:00".into());
        assert!(validate(&input, now).is_err());
        input.starts_at = None;
        input.expires_at = Some("2026-10-09T00:00:00Z".into());
        assert!(validate(&input, now).is_err());
        input.expires_at = None;
        for name in ["".into(), " ".into(), "名".repeat(65), "ab\nc".into()] {
            input.name = name;
            assert!(validate(&input, now).is_err());
        }
    }
}
