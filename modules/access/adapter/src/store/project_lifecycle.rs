use super::{PostgresAccess, db_error, management::require_manager, uuid};
use async_trait::async_trait;
use nexofolio_access_contracts::SessionPrincipal;
use nexofolio_common::{Error, ProjectId, Result};
use nexofolio_contracts::project_lifecycle::{
    PROJECT_CONTENT_LOCK, ProjectActivity, ProjectLifecycle,
};
use sqlx::{PgConnection, Postgres, Transaction};

struct Activity {
    _transaction: Transaction<'static, Postgres>,
}
impl ProjectActivity for Activity {}

#[async_trait]
impl ProjectLifecycle for PostgresAccess {
    async fn activity(&self, project: ProjectId) -> Result<Box<dyn ProjectActivity>> {
        let mut tx = self
            .database
            .activity_pool
            .begin()
            .await
            .map_err(db_error)?;
        let locked: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock_shared($1,hashtext($2))")
                .bind(PROJECT_CONTENT_LOCK)
                .bind(project.to_string())
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        if !locked {
            return Err(Error::Conflict);
        }
        Ok(Box::new(Activity { _transaction: tx }))
    }
}

/// Caller-owned transaction for composition. Participating adapters access only their own schema.
/// No pool or SQL execution is exposed to the HTTP layer.
pub struct ProjectReset {
    transaction: Transaction<'static, Postgres>,
}
impl ProjectReset {
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.transaction
    }
    pub async fn commit(self) -> Result<()> {
        self.transaction.commit().await.map_err(db_error)
    }
}

impl PostgresAccess {
    pub async fn begin_project_reset(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        confirmation: Option<&str>,
    ) -> Result<ProjectReset> {
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        require_manager(&mut tx, principal).await?;
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_CONTENT_LOCK)
            .bind(project.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
        if !locked {
            return Err(Error::Conflict);
        }
        let name: String =
            sqlx::query_scalar("SELECT name FROM projects WHERE id=$1 AND instance=$2 FOR UPDATE")
                .bind(uuid(project))
                .bind(&principal.instance)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?
                .ok_or(Error::NotFound)?;
        if confirmation.is_some_and(|value| value != name) {
            return Err(Error::invalid("confirmation does not match project"));
        }
        Ok(ProjectReset { transaction: tx })
    }
}
