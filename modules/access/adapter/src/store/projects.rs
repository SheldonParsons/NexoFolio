use super::{PostgresAccess, db_error, typed, uuid};
use async_trait::async_trait;
use nexofolio_access_contracts::{
    AccessState, ProjectAccess, ProjectCard, ProjectPage, SessionPrincipal,
};
use nexofolio_common::{Error, ProjectId, Result};
use sqlx::{Row, postgres::PgRow};

#[async_trait]
impl ProjectAccess for PostgresAccess {
    async fn list_projects(
        &self,
        p: &SessionPrincipal,
        page: u32,
        limit: u32,
    ) -> Result<ProjectPage> {
        if page == 0 || page > 100000 || !(1..=100).contains(&limit) {
            return Err(Error::invalid("page must be 1..100000 and limit 1..100"));
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE instance=$1")
            .bind(&p.instance)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        let rows = sqlx::query(
            r#"SELECT p.id,p.name,p.status,u.grants_synced,EXISTS(SELECT 1
            FROM user_project_access a
            WHERE a.project_id=p.id
            AND a.user_id=u.id) AS allowed
            FROM projects p
            JOIN users u ON u.id=$1
            AND u.instance=p.instance
            AND u.enabled
            WHERE p.instance=$2
            ORDER BY p.id
            LIMIT $3 OFFSET $4"#,
        )
        .bind(uuid(p.user_id))
        .bind(&p.instance)
        .bind(i64::from(limit))
        .bind(i64::from((page - 1) * limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let items = rows.iter().map(card).collect::<Result<Vec<_>>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(ProjectPage {
            items,
            page,
            limit,
            total: total as u64,
        })
    }
    async fn require_project(&self, p: &SessionPrincipal, id: ProjectId) -> Result<ProjectCard> {
        let row = sqlx::query(
            r#"SELECT p.id,p.name,p.status,u.grants_synced,EXISTS(SELECT 1
            FROM user_project_access a
            WHERE a.project_id=p.id
            AND a.user_id=u.id) AS allowed
            FROM projects p
            JOIN users u ON u.id=$1
            AND u.instance=p.instance
            AND u.enabled
            WHERE p.instance=$2
            AND p.id=$3"#,
        )
        .bind(uuid(p.user_id))
        .bind(&p.instance)
        .bind(uuid(id))
        .fetch_optional(&self.database.pool)
        .await
        .map_err(db_error)?
        .ok_or(Error::NotFound)?;
        let item = card(&row)?;
        match item.access_state {
            AccessState::Allowed => Ok(item),
            AccessState::Denied => Err(Error::Forbidden),
            AccessState::Unknown => Err(Error::Unavailable {
                component: "project_access",
            }),
        }
    }
}

fn card(row: &PgRow) -> Result<ProjectCard> {
    let synced: bool = row.try_get("grants_synced").map_err(db_error)?;
    let allowed: bool = row.try_get("allowed").map_err(db_error)?;
    let (access_state, reason_code) = if !synced {
        (AccessState::Unknown, Some("PROJECT_ACCESS_UNAVAILABLE"))
    } else if allowed {
        (AccessState::Allowed, None)
    } else {
        (AccessState::Denied, Some("PROJECT_ACCESS_DENIED"))
    };
    Ok(ProjectCard {
        project_id: typed(row.try_get("id").map_err(db_error)?),
        name: row.try_get("name").map_err(db_error)?,
        status: row.try_get("status").map_err(db_error)?,
        can_access: synced && allowed,
        access_state,
        reason_code,
    })
}
