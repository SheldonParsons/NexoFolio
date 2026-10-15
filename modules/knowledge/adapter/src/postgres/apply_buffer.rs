//! Only formal candidate application opts in. The command interpreter still
//! creates every revision and protected write; this adapter batches their IO.
use super::{
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::{FieldNote, KnowledgeSnapshot, Revision};
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::types::Json;
use std::collections::{HashMap, HashSet};

pub(super) struct ApplyBuffer {
    pub project: ProjectId,
    pub owned: HashSet<EndpointId>,
    pub fields: HashMap<(EndpointId, String), FieldNote>,
    pub pending: HashMap<(EndpointId, String), Option<FieldNote>>,
    pub revisions: Vec<Revision>,
}
impl ApplyBuffer {
    pub fn new(project: ProjectId, view: &KnowledgeSnapshot) -> Self {
        Self {
            project,
            owned: view.endpoints.iter().copied().collect(),
            fields: view
                .fields
                .iter()
                .map(|f| ((f.endpoint, f.path.clone()), f.clone()))
                .collect(),
            pending: HashMap::new(),
            revisions: Vec::new(),
        }
    }
}
impl PostgresTx {
    pub(super) async fn flush_apply(&mut self) -> StoreResult<()> {
        let Some(buffer) = &mut self.apply_buffer else {
            return Ok(());
        };
        if buffer.revisions.is_empty() && buffer.pending.is_empty() {
            return Ok(());
        }
        // Revisions enter the same transaction before any of their field writes.
        sqlx::query("INSERT INTO revisions(round_id,seq,target,before) SELECT round,seq,target,before FROM jsonb_to_recordset($1) AS r(round uuid,seq bigint,target jsonb,before jsonb)")
            .bind(Json(&buffer.revisions)).execute(&mut *self.tx).await.map_err(unavailable)?;
        let writes: Vec<_> = buffer.pending.values().filter_map(Option::as_ref).collect();
        if !writes.is_empty() {
            sqlx::query(r#"INSERT INTO field_notes(endpoint_id,path,project_id,text,values_seen,context,author,at) SELECT endpoint,path,$2,text,coalesce(f."values",'[]'::jsonb),context,author,at FROM jsonb_to_recordset($1) AS f(endpoint uuid,path text,text text,"values" jsonb,context jsonb,author jsonb,at timestamptz) ON CONFLICT(endpoint_id,path) DO UPDATE SET text=excluded.text,values_seen=excluded.values_seen,context=excluded.context,author=excluded.author,at=excluded.at"#)
                .bind(Json(writes)).bind(uuid(buffer.project)).execute(&mut *self.tx).await.map_err(unavailable)?;
        }
        let removed: Vec<_> = buffer
            .pending
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|((endpoint, path), _)| serde_json::json!({"endpoint":endpoint,"path":path}))
            .collect();
        if !removed.is_empty() {
            sqlx::query("DELETE FROM field_notes f USING jsonb_to_recordset($1) AS d(endpoint uuid,path text) WHERE f.project_id=$2 AND f.endpoint_id=d.endpoint AND f.path=d.path")
                .bind(Json(removed)).bind(uuid(buffer.project)).execute(&mut *self.tx).await.map_err(unavailable)?;
        }
        buffer.pending.clear();
        buffer.revisions.clear();
        Ok(())
    }
}
