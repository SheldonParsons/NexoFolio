//! Observe-owned reset SQL; project configuration and idempotency tombstones survive.
use crate::PostgresObserve;
use nexofolio_common::{Error, ProjectId, Result};
use nexofolio_contracts::project_lifecycle::ProjectContentCounts;
use sqlx::{PgConnection, Row};

fn unavailable(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "observe_reset",
    }
}

impl PostgresObserve {
    pub async fn prepare_project_reset(
        &self,
        connection: &mut PgConnection,
        project: ProjectId,
    ) -> Result<ProjectContentCounts> {
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(crate::postgres::PROJECT_LOCK)
            .bind(project.to_string())
            .execute(&mut *connection)
            .await
            .map_err(unavailable)?;
        let id: uuid::Uuid = project.to_string().parse().expect("UUID");
        let row=sqlx::query("SELECT (SELECT count(*) FROM observe.endpoints WHERE project_id=$1 AND merged_into IS NULL) AS interfaces, (SELECT count(*) FROM observe.fingerprints f JOIN observe.endpoints e ON e.id=f.endpoint_id WHERE e.project_id=$1) AS structures")
            .bind(id).fetch_one(connection).await.map_err(unavailable)?;
        Ok(ProjectContentCounts {
            interfaces: row.get::<i64, _>("interfaces") as u64,
            structures: row.get::<i64, _>("structures") as u64,
            ..Default::default()
        })
    }

    pub async fn reset_project_content(
        &self,
        connection: &mut PgConnection,
        project: ProjectId,
    ) -> Result<()> {
        let id: uuid::Uuid = project.to_string().parse().expect("UUID");
        for query in [
            "DELETE FROM observe.outbox WHERE event->>'project_id'=($1::uuid)::text",
            "DELETE FROM observe.fingerprints WHERE endpoint_id IN(SELECT id FROM observe.endpoints WHERE project_id=$1)",
            "DELETE FROM observe.declarations WHERE endpoint_id IN(SELECT id FROM observe.endpoints WHERE project_id=$1)",
            "DELETE FROM observe.endpoints WHERE project_id=$1",
            "UPDATE observe.service_addresses SET calls=0,last_seen=NULL WHERE project_id=$1",
            "INSERT INTO observe.project_resets(project_id,cleared_at) VALUES($1,clock_timestamp()) ON CONFLICT(project_id) DO UPDATE SET cleared_at=excluded.cleared_at",
        ] {
            sqlx::query(query)
                .bind(id)
                .execute(&mut *connection)
                .await
                .map_err(unavailable)?;
        }
        Ok(())
    }
}
