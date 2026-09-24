use super::{PostgresAccess, db_error, typed, uuid};
use async_trait::async_trait;
use nexofolio_access_contracts::{
    Environment, EnvironmentPage, Environments, ProjectAccess, SessionPrincipal,
    validate_environment_name,
};
use nexofolio_common::{EnvironmentId, Error, ProjectId, Result};
use sqlx::{Postgres as Pg, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

#[async_trait]
impl Environments for PostgresAccess {
    async fn list_environments(
        &self,
        p: &SessionPrincipal,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<EnvironmentPage> {
        self.require_project(p, project).await?;
        if page == 0 || page > 100000 || !(1..=100).contains(&limit) {
            return Err(Error::invalid("invalid pagination"));
        }
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM environments WHERE project_id=$1")
                .bind(uuid(project))
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let rows=sqlx::query("SELECT id,name FROM environments WHERE project_id=$1 ORDER BY name,id LIMIT $2 OFFSET $3").bind(uuid(project)).bind(i64::from(limit)).bind(i64::from((page-1)*limit)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let items = rows.iter().map(from_row).collect::<Result<_>>()?;
        tx.commit().await.map_err(db_error)?;
        Ok(EnvironmentPage {
            items,
            total: total as u64,
            page,
            limit,
        })
    }
    async fn create_environment(
        &self,
        p: &SessionPrincipal,
        project: ProjectId,
        name: &str,
    ) -> Result<Environment> {
        self.require_project(p, project).await?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        lock_environment_access(&mut tx, p, project).await?;
        let result = by_name(&mut tx, uuid(project), name).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
    async fn rename_environment(
        &self,
        p: &SessionPrincipal,
        project: ProjectId,
        id: EnvironmentId,
        name: &str,
    ) -> Result<Environment> {
        self.require_project(p, project).await?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        lock_environment_access(&mut tx, p, project).await?;
        let result = rename(&mut tx, uuid(project), uuid(id), name).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }
}

/// Holds the user and re-checks the grant inside the writing transaction.
async fn lock_environment_access(
    tx: &mut Transaction<'_, Pg>,
    p: &SessionPrincipal,
    project: ProjectId,
) -> Result<()> {
    let user =
        sqlx::query("SELECT id FROM users WHERE id=$1 AND instance=$2 AND enabled FOR SHARE")
            .bind(uuid(p.user_id))
            .bind(&p.instance)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
    if user.is_none() {
        return Err(Error::Unauthenticated);
    }
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_project_access a JOIN projects p ON a.project_id=p.id WHERE a.user_id=$1 AND a.project_id=$2 AND p.instance=$3)").bind(uuid(p.user_id)).bind(uuid(project)).bind(&p.instance).fetch_one(&mut **tx).await.map_err(db_error)?;
    if !allowed {
        return Err(Error::Forbidden);
    }
    Ok(())
}

fn from_row(row: &PgRow) -> Result<Environment> {
    Ok(Environment {
        id: typed(row.try_get("id").map_err(db_error)?),
        name: row.try_get("name").map_err(db_error)?,
    })
}

/// Serialises every name change within one project.
async fn lock_names(tx: &mut Transaction<'_, Pg>, project: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("environment-names:{project}"))
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    Ok(())
}

pub(crate) async fn by_id(
    tx: &mut Transaction<'_, Pg>,
    project: Uuid,
    id: Uuid,
) -> Result<Option<Environment>> {
    sqlx::query("SELECT id,name FROM environments WHERE project_id=$1 AND id=$2")
        .bind(project)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?
        .as_ref()
        .map(from_row)
        .transpose()
}

/// Current names and old aliases both resolve; an unknown name creates the environment.
pub(crate) async fn by_name(
    tx: &mut Transaction<'_, Pg>,
    project: Uuid,
    name: &str,
) -> Result<Environment> {
    validate_environment_name(name)?;
    let find = async |tx: &mut Transaction<'_, Pg>| {
        sqlx::query("SELECT e.id,e.name FROM environment_names n JOIN environments e ON e.id=n.environment_id WHERE n.project_id=$1 AND n.name=$2")
            .bind(project).bind(name).fetch_optional(&mut **tx).await.map_err(db_error)
    };
    if let Some(row) = find(tx).await? {
        return from_row(&row);
    }
    lock_names(tx, project).await?;
    if let Some(row) = find(tx).await? {
        return from_row(&row);
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,$3)")
        .bind(id)
        .bind(project)
        .bind(name)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    sqlx::query("INSERT INTO environment_names(project_id,name,environment_id) VALUES($1,$2,$3)")
        .bind(project)
        .bind(name)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    Ok(Environment {
        id: typed(id),
        name: name.to_owned(),
    })
}

async fn rename(
    tx: &mut Transaction<'_, Pg>,
    project: Uuid,
    id: Uuid,
    name: &str,
) -> Result<Environment> {
    validate_environment_name(name)?;
    lock_names(tx, project).await?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT environment_id FROM environment_names WHERE project_id=$1 AND name=$2",
    )
    .bind(project)
    .bind(name)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?;
    if existing.is_some_and(|v| v != id) {
        return Err(Error::Conflict);
    }
    let row = sqlx::query(
        "UPDATE environments SET name=$1 WHERE id=$2 AND project_id=$3 RETURNING id,name",
    )
    .bind(name)
    .bind(id)
    .bind(project)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    .ok_or(Error::NotFound)?;
    sqlx::query("INSERT INTO environment_names(project_id,name,environment_id) VALUES($1,$2,$3) ON CONFLICT(project_id,name) DO NOTHING").bind(project).bind(name).bind(id).execute(&mut **tx).await.map_err(db_error)?;
    from_row(&row)
}
