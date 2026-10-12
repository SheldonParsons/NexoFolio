use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::DocumentLifecycle;
use nexofolio_knowledge_contracts::StoreResult;
impl PostgresTx {
    pub(super) async fn db_lifecycle(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<DocumentLifecycle> {
        let state: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT state FROM document_lifecycle WHERE project_id=$1")
                .bind(uuid(project))
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(unavailable)?;
        state
            .map(serde_json::from_value)
            .transpose()
            .map(|s| s.unwrap_or_default())
            .map_err(unavailable)
    }
    pub(super) async fn db_put_lifecycle(
        &mut self,
        project: ProjectId,
        state: &DocumentLifecycle,
    ) -> StoreResult<()> {
        let value = serde_json::to_value(state).map_err(unavailable)?;
        sqlx::query("INSERT INTO document_lifecycle (project_id,state) VALUES ($1,$2) ON CONFLICT (project_id) DO UPDATE SET state=EXCLUDED.state")
            .bind(uuid(project)).bind(&value).execute(&mut *self.tx).await.map_err(unavailable)?;
        sqlx::query("INSERT INTO document_lifecycle_history (project_id,version,state) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(uuid(project)).bind(state.version as i64).bind(value).execute(&mut *self.tx).await.map_err(unavailable)?;
        Ok(())
    }
}
