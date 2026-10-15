//! Read only progress and execution metadata; do not load snapshots or captured samples.
use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::CandidateDiagnostics;
use nexofolio_knowledge_contracts::StoreResult;
impl PostgresTx {
    pub(super) async fn db_candidate_diagnostics(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<CandidateDiagnostics>> {
        let value:Option<serde_json::Value>=sqlx::query_scalar("SELECT jsonb_build_object('summary',jsonb_build_object('id',id,'project_id',project_id,'target',target,'state',state,'created_at',created_at,'updated_at',updated_at,'commands',coalesce(progress->'retained_commands',(SELECT to_jsonb(coalesce(sum(jsonb_array_length(b->'commands')),0)) FROM jsonb_array_elements(batches) b)),'progress',(CASE WHEN state='running' THEN progress || jsonb_build_object('heartbeat_at',updated_at) ELSE progress END) || jsonb_build_object('history_available',history_pruned_at IS NULL)),'batches',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',b->'id','author',b->'author','state',b->'state','commands',jsonb_array_length(b->'commands'))),'[]'::jsonb) FROM jsonb_array_elements(batches) b),'operations',operations) FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project)).bind(id).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        value
            .map(|value| serde_json::from_value(value).map_err(unavailable))
            .transpose()
    }
}
