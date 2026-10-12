//! Explicit public metadata read, separate from member/MCP authorization.
use crate::Postgres;
use async_trait::async_trait;
use nexofolio_access_contracts::{DocumentProject, Environment, ProjectDocuments};
use nexofolio_common::{Error, ProjectId, Result};
use sqlx::Row;
use uuid::Uuid;

fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "access_database",
    }
}

#[async_trait]
impl ProjectDocuments for Postgres {
    async fn document_project(
        &self,
        instance: &str,
        project: ProjectId,
    ) -> Result<Option<DocumentProject>> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let id: Uuid = project.to_string().parse().expect("typed UUID");
        let Some(row) =
            sqlx::query("SELECT id,name,status FROM projects WHERE id=$1 AND instance=$2")
                .bind(id)
                .bind(instance)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?
        else {
            return Ok(None);
        };
        let rows =
            sqlx::query("SELECT id,name FROM environments WHERE project_id=$1 ORDER BY name,id")
                .bind(id)
                .fetch_all(&mut *tx)
                .await
                .map_err(db_error)?;
        let environments = rows
            .iter()
            .map(|r| {
                Ok(Environment {
                    id: r
                        .try_get::<Uuid, _>("id")
                        .map_err(db_error)?
                        .to_string()
                        .parse()
                        .expect("UUID"),
                    name: r.try_get("name").map_err(db_error)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let result = DocumentProject {
            project_id: project,
            name: row.try_get("name").map_err(db_error)?,
            status: row.try_get("status").map_err(db_error)?,
            environments,
        };
        tx.commit().await.map_err(db_error)?;
        Ok(Some(result))
    }
}
