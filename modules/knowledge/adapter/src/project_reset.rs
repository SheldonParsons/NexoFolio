//! Knowledge-owned participation in the caller's atomic project reset transaction.
use crate::PostgresKnowledge;
use nexofolio_common::{Error, ProjectId, Result};
use nexofolio_contracts::project_lifecycle::ProjectContentCounts;
use sqlx::{PgConnection, Row};

fn unavailable(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "knowledge_reset",
    }
}

impl PostgresKnowledge {
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
        let row=sqlx::query("SELECT (SELECT count(*) FROM knowledge.folders WHERE project_id=$1) AS folders, (SELECT count(*) FROM knowledge.candidates WHERE project_id=$1) AS candidates, (SELECT count(*) FROM knowledge.candidates WHERE project_id=$1 AND state='running') AS running")
            .bind(id).fetch_one(connection).await.map_err(unavailable)?;
        Ok(ProjectContentCounts {
            folders: row.get::<i64, _>("folders") as u64,
            curation_results: row.get::<i64, _>("candidates") as u64,
            running_curations: row.get::<i64, _>("running") as u64,
            ..Default::default()
        })
    }

    pub async fn reset_project_content(
        &self,
        connection: &mut PgConnection,
        project: ProjectId,
    ) -> Result<()> {
        let id: uuid::Uuid = project.to_string().parse().expect("UUID");
        // Fail closed if legacy corruption would make an FK cascade cross project boundaries.
        let crosses_project: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge.folders child JOIN knowledge.folders parent ON parent.id=child.parent_id WHERE parent.project_id=$1 AND child.project_id<>$1) OR EXISTS(SELECT 1 FROM knowledge.placements p JOIN knowledge.folders f ON f.id=p.folder_id WHERE f.project_id=$1 AND p.project_id<>$1) OR EXISTS(SELECT 1 FROM knowledge.field_value_archives a JOIN knowledge.candidates c ON c.id=a.candidate_id WHERE c.project_id=$1 AND a.project_id<>$1)")
            .bind(id).fetch_one(&mut *connection).await.map_err(unavailable)?;
        if crosses_project {
            return Err(Error::Unavailable {
                component: "knowledge_reset_scope",
            });
        }
        // Parent links and archives belong to the same project; round revisions cascade.
        for query in [
            "DELETE FROM knowledge.document_lifecycle_history WHERE project_id=$1",
            "DELETE FROM knowledge.document_lifecycle WHERE project_id=$1",
            "UPDATE knowledge.candidates SET parent=NULL WHERE project_id=$1",
            "DELETE FROM knowledge.field_value_archives WHERE project_id=$1",
            "DELETE FROM knowledge.candidates WHERE project_id=$1",
            "DELETE FROM knowledge.links WHERE project_id=$1",
            "DELETE FROM knowledge.field_notes WHERE project_id=$1",
            "DELETE FROM knowledge.endpoint_notes WHERE project_id=$1",
            "DELETE FROM knowledge.placements WHERE project_id=$1",
            "DELETE FROM knowledge.folders WHERE project_id=$1",
            "DELETE FROM knowledge.rounds WHERE project_id=$1",
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
