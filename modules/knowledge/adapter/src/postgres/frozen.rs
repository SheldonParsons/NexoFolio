//! Opaque immutable input blocks; content hashes are verified by knowledge core.
use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_knowledge_contracts::StoreResult;

impl PostgresTx {
    pub(super) async fn db_candidate_input(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<serde_json::Value>> {
        Ok(sqlx::query_scalar::<_, Option<serde_json::Value>>(
            "SELECT CASE WHEN evidence_input IS NOT NULL THEN evidence_input WHEN execution_input->>'version'='3' THEN execution_input WHEN execution_input IS NOT NULL THEN jsonb_build_object('version',execution_input->'version') END FROM candidates WHERE project_id=$1 AND id=$2",
        )
        .bind(uuid(project))
        .bind(candidate)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .flatten())
    }
    pub(super) async fn db_frozen_chunk(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        hash: &str,
    ) -> StoreResult<Option<String>> {
        sqlx::query_scalar("SELECT content FROM frozen_input_chunks WHERE project_id=$1 AND candidate_id=$2 AND snapshot_id=$3 AND sha256=$4")
            .bind(uuid(project)).bind(candidate).bind(snapshot).bind(hash).fetch_optional(&mut *self.tx).await.map_err(unavailable)
    }
    pub(super) async fn db_put_frozen_chunk(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        hash: &str,
        content: &str,
    ) -> StoreResult<()> {
        self.db_lock_project(project).await?;
        // Only field-input arrays carry archive IDs. Source documents remain opaque.
        if content.trim_start().starts_with('[')
            && sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM retention_projects WHERE project_id=$1)",
            )
            .bind(uuid(project))
            .fetch_one(&mut *self.tx)
            .await
            .map_err(unavailable)?
        {
            let fields: Vec<serde_json::Value> =
                serde_json::from_str(content).map_err(unavailable)?;
            let refs = super::retention::input_refs(&fields)?;
            let expired: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidates WHERE project_id=$1 AND id=ANY($2) AND archive_reclaimed_at IS NOT NULL)")
                .bind(uuid(project)).bind(&refs).fetch_one(&mut *self.tx).await.map_err(unavailable)?;
            if expired {
                return Err(nexofolio_knowledge_contracts::StoreError::HistoryExpired);
            }
            sqlx::query("UPDATE candidates SET archive_unreferenced_since=NULL WHERE project_id=$1 AND id=ANY($2) AND archive_unreferenced_since IS NOT NULL")
                .bind(uuid(project)).bind(&refs).execute(&mut *self.tx).await.map_err(unavailable)?;
        }
        sqlx::query("INSERT INTO frozen_input_chunks(project_id,candidate_id,snapshot_id,sha256,content) VALUES($1,$2,$3,$4,$5)")
            .bind(uuid(project)).bind(candidate).bind(snapshot).bind(hash).bind(content).execute(&mut *self.tx).await.map_err(unavailable)?;
        Ok(())
    }
}
