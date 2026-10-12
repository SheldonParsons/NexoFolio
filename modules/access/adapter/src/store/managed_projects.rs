//! Transactional local-project metadata. Member grants are always derived, never copied.
use super::{PostgresAccess, db_error, management::require_manager, typed, uuid};
use nexofolio_access_contracts::{
    ManagedProject, PermissionSource, SessionPrincipal, validate_permission_sources,
    validate_project_name,
};
use nexofolio_common::{Error, ProjectId, Result};
use nexofolio_contracts::project_lifecycle::ProjectLifecycle;
use sqlx::{PgConnection, Row};

async fn sources(
    connection: &mut PgConnection,
    instance: &str,
    ids: &[ProjectId],
) -> Result<Vec<PermissionSource>> {
    let ids: Vec<_> = ids.iter().map(uuid).collect();
    let rows = sqlx::query("SELECT id,name,status FROM projects WHERE instance=$1 AND origin='zentao' AND id=ANY($2) ORDER BY id FOR KEY SHARE")
        .bind(instance).bind(&ids).fetch_all(connection).await.map_err(db_error)?;
    if rows.len() != ids.len() {
        return Err(Error::NotFound);
    }
    Ok(rows
        .into_iter()
        .map(|row| PermissionSource {
            project_id: typed(row.get("id")),
            name: row.get("name"),
            status: row.get("status"),
        })
        .collect())
}

async fn bind_sources(
    connection: &mut PgConnection,
    principal: &SessionPrincipal,
    project: ProjectId,
    ids: &[ProjectId],
) -> Result<()> {
    sqlx::query("DELETE FROM project_permission_sources WHERE project_id=$1")
        .bind(uuid(project))
        .execute(&mut *connection)
        .await
        .map_err(db_error)?;
    let ids: Vec<_> = ids.iter().map(uuid).collect();
    sqlx::query("INSERT INTO project_permission_sources(instance,project_id,source_project_id) SELECT $1,$2,unnest($3::uuid[])")
        .bind(&principal.instance).bind(uuid(project)).bind(ids).execute(connection).await.map_err(db_error)?;
    Ok(())
}

fn managed(
    project: ProjectId,
    name: String,
    revision: i64,
    sources: Vec<PermissionSource>,
) -> Result<ManagedProject> {
    let permission_source = sources.first().cloned().ok_or(Error::Unavailable {
        component: "project_permission_sources",
    })?;
    Ok(ManagedProject {
        project_id: project,
        name,
        revision,
        permission_source,
        permission_sources: sources,
    })
}
fn write_error(error: sqlx::Error) -> Error {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        Error::Conflict
    } else {
        db_error(error)
    }
}

impl PostgresAccess {
    pub async fn managed_project(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
    ) -> Result<ManagedProject> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        let row = sqlx::query("SELECT name,management_revision FROM projects WHERE id=$1 AND instance=$2 AND origin='local'")
            .bind(uuid(project)).bind(&principal.instance).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or(Error::NotFound)?;
        let ids: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT source_project_id FROM project_permission_sources WHERE project_id=$1 ORDER BY source_project_id")
            .bind(uuid(project)).fetch_all(&mut *tx).await.map_err(db_error)?;
        let ids: Vec<ProjectId> = ids.into_iter().map(typed).collect();
        let sources = sources(&mut tx, &principal.instance, &ids).await?;
        let result = managed(
            project,
            row.get("name"),
            row.get("management_revision"),
            sources,
        )?;
        tx.commit().await.map_err(db_error)?;
        Ok(result)
    }

    pub async fn create_local_project(
        &self,
        principal: &SessionPrincipal,
        name: &str,
        ids: &[ProjectId],
    ) -> Result<ManagedProject> {
        validate_project_name(name)?;
        validate_permission_sources(ids)?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        let sources = sources(&mut tx, &principal.instance, ids).await?;
        let project = ProjectId::new();
        sqlx::query(
            "INSERT INTO projects(id,instance,name,status,origin) VALUES($1,$2,$3,'doing','local')",
        )
        .bind(uuid(project))
        .bind(&principal.instance)
        .bind(name)
        .execute(&mut *tx)
        .await
        .map_err(write_error)?;
        bind_sources(&mut tx, principal, project, ids).await?;
        tx.commit().await.map_err(db_error)?;
        managed(project, name.into(), 1, sources)
    }

    pub async fn edit_local_project(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        name: &str,
        ids: &[ProjectId],
        revision: i64,
    ) -> Result<ManagedProject> {
        validate_project_name(name)?;
        validate_permission_sources(ids)?;
        if revision < 1 || revision == i64::MAX {
            return Err(Error::invalid("invalid project revision"));
        }
        // Metadata edits may coexist with curation, but cannot race a content reset.
        let _activity = self.activity(project).await?;
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        let current: i64 = sqlx::query_scalar("SELECT management_revision FROM projects WHERE id=$1 AND instance=$2 AND origin='local' FOR UPDATE")
            .bind(uuid(project)).bind(&principal.instance).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or(Error::NotFound)?;
        if current != revision {
            return Err(Error::Conflict);
        }
        let sources = sources(&mut tx, &principal.instance, ids).await?;
        sqlx::query(
            "UPDATE projects SET name=$1,management_revision=management_revision+1 WHERE id=$2",
        )
        .bind(name)
        .bind(uuid(project))
        .execute(&mut *tx)
        .await
        .map_err(write_error)?;
        bind_sources(&mut tx, principal, project, ids).await?;
        tx.commit().await.map_err(db_error)?;
        managed(project, name.into(), revision + 1, sources)
    }
}
