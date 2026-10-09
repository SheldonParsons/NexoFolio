use super::{
    codec::{id, json, parse, text, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{Candidate, CandidateEntryKind, CandidateSummary};
use nexofolio_knowledge_contracts::StoreResult;
use sqlx::{Row, postgres::PgRow};

fn candidate(row: &PgRow) -> StoreResult<Candidate> {
    Ok(Candidate {
        id: row.get("id"),
        project_id: id(row.get("project_id")),
        target: row.get::<Option<uuid::Uuid>, _>("target").map(id),
        state: parse(row.get("state"))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        base_hash: row.get("base_hash"),
        base: serde_json::from_value(row.get("base")).map_err(unavailable)?,
        view: serde_json::from_value(row.get("view")).map_err(unavailable)?,
        batches: serde_json::from_value(row.get("batches")).map_err(unavailable)?,
        progress: row.get("progress"),
        execution_input: row.get("execution_input"),
        parent: row.get("parent"),
        applied_rounds: serde_json::from_value(row.get("applied_rounds")).map_err(unavailable)?,
        reviewed_by: row.get::<Option<uuid::Uuid>, _>("reviewed_by").map(id),
        operations: serde_json::from_value(row.get("operations")).map_err(unavailable)?,
    })
}
fn summary(row: &PgRow) -> StoreResult<CandidateSummary> {
    let mut progress: serde_json::Value = row.get("progress");
    if progress.is_object() {
        if row.get::<&str, _>("state") == "running" {
            progress["heartbeat_at"] =
                serde_json::json!(row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"));
        }
        progress["resume_available"] = serde_json::json!(
            matches!(
                row.get::<&str, _>("state"),
                "running" | "partial" | "interrupted"
            ) && row.get::<bool, _>("input_frozen")
        );
    }
    Ok(CandidateSummary {
        id: row.get("id"),
        project_id: id(row.get("project_id")),
        target: row.get::<Option<uuid::Uuid>, _>("target").map(id),
        state: parse(row.get("state"))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        progress,
        commands: row.get::<i64, _>("commands") as usize,
    })
}
impl PostgresTx {
    pub(super) async fn db_candidate_operations(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateOperationHistory>> {
        let row=sqlx::query("SELECT id,project_id,state,created_at,updated_at,reviewed_by,operations FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project)).bind(key).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        row.as_ref()
            .map(|r| {
                Ok(nexofolio_contracts::knowledge::CandidateOperationHistory {
                    id: r.get("id"),
                    project_id: id(r.get("project_id")),
                    state: parse(r.get("state"))?,
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                    reviewed_by: r.get::<Option<uuid::Uuid>, _>("reviewed_by").map(id),
                    operations: serde_json::from_value(r.get("operations")).map_err(unavailable)?,
                })
            })
            .transpose()
    }
    pub(super) async fn db_active_candidate(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Option<CandidateSummary>> {
        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,coalesce(execution_input->>'version'='2',false) AS input_frozen,progress,(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0)::bigint FROM jsonb_array_elements(batches) b) AS commands FROM candidates WHERE project_id=$1 AND state='running' ORDER BY created_at LIMIT 1")
            .bind(uuid(project)).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        row.as_ref().map(summary).transpose()
    }
    pub(super) async fn db_candidate_owner(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<uuid::Uuid>> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT progress->>'run_id' FROM candidates WHERE project_id=$1 AND id=$2",
        )
        .bind(uuid(project))
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?
        .flatten();
        value.map(|s| s.parse().map_err(unavailable)).transpose()
    }
    pub(super) async fn db_heartbeat(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<()> {
        sqlx::query(
            "UPDATE candidates SET updated_at=$3 WHERE project_id=$1 AND id=$2 AND state='running'",
        )
        .bind(uuid(project))
        .bind(id)
        .bind(at)
        .execute(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        Ok(())
    }

    pub(super) async fn db_candidate_state(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateState>> {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM candidates WHERE project_id=$1 AND id=$2")
                .bind(uuid(project))
                .bind(id)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(unavailable)?;
        state.map(parse).transpose()
    }
    pub(super) async fn db_checkpoint_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        state: nexofolio_contracts::knowledge::CandidateState,
        progress: &serde_json::Value,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<()> {
        let result = sqlx::query("UPDATE candidates SET state=$3,progress=$4,updated_at=$5 WHERE project_id=$1 AND id=$2 AND state='running'")
            .bind(uuid(project)).bind(id).bind(text(state)).bind(progress).bind(at).execute(&mut *self.tx).await.map_err(unavailable)?;
        if result.rows_affected() != 1 {
            return Err(nexofolio_knowledge_contracts::StoreError::Unavailable);
        }
        Ok(())
    }
    pub(super) async fn db_candidate(
        &mut self,
        project: ProjectId,
        candidate_id: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        let row = sqlx::query("SELECT * FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project))
            .bind(candidate_id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        row.as_ref().map(candidate).transpose()
    }
    pub(super) async fn db_candidate_projection(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,operations,NULL::jsonb AS execution_input FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project)).bind(key).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        row.as_ref().map(candidate).transpose()
    }
    pub(super) async fn db_put_candidate_projection(&mut self, c: &Candidate) -> StoreResult<()> {
        let result=sqlx::query("UPDATE candidates SET state=$3,updated_at=$4,view=$5,batches=$6,progress=$7,applied_rounds=$8,reviewed_by=$9,operations=$10 WHERE project_id=$1 AND id=$2")
            .bind(uuid(c.project_id)).bind(c.id).bind(text(c.state)).bind(c.updated_at)
            .bind(json(&c.view)?).bind(json(&c.batches)?).bind(&c.progress)
            .bind(json(&c.applied_rounds)?).bind(c.reviewed_by.map(uuid)).bind(json(&c.operations)?)
            .execute(&mut *self.tx).await.map_err(unavailable)?;
        if result.rows_affected() != 1 {
            return Err(nexofolio_knowledge_contracts::StoreError::Unavailable);
        }
        Ok(())
    }
    pub(super) async fn db_candidates(
        &mut self,
        project: ProjectId,
        limit: usize,
    ) -> StoreResult<Vec<CandidateSummary>> {
        let rows = sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,coalesce(execution_input->>'version'='2',false) AS input_frozen,progress,(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0)::bigint FROM jsonb_array_elements(batches) b) AS commands FROM candidates WHERE project_id=$1 ORDER BY created_at DESC LIMIT $2")
            .bind(uuid(project)).bind(limit as i64).fetch_all(&mut *self.tx).await.map_err(unavailable)?;
        rows.iter().map(summary).collect()
    }
    pub(super) async fn db_entry_candidates(
        &mut self,
        project: ProjectId,
        kind: CandidateEntryKind,
        selected: Option<uuid::Uuid>,
    ) -> StoreResult<Vec<CandidateSummary>> {
        let rows = sqlx::query(
            r#"
            WITH preferred AS (
                SELECT id FROM candidates WHERE project_id=$1
                ORDER BY CASE
                    WHEN $2 AND state='running' THEN 0
                    WHEN state IN ('ready','partial') OR ($2 AND state='interrupted') THEN 1
                    WHEN $2 AND state='applied' THEN 2
                    WHEN $2 AND state='discarded' THEN 3 ELSE 4 END,
                    created_at DESC,id DESC LIMIT 1)
            SELECT id,project_id,target,state,created_at,updated_at,coalesce(execution_input->>'version'='2',false) AS input_frozen,
                (CASE WHEN jsonb_typeof(progress)='object' THEN progress ELSE '{}'::jsonb END)
                || jsonb_build_object('folders_saved', coalesce(nullif(progress->'folders_saved','null'::jsonb),
                    to_jsonb(jsonb_array_length(view->'folders')))) AS progress,
                (SELECT coalesce(sum(jsonb_array_length(b->'commands')),0)::bigint
                 FROM jsonb_array_elements(batches) b) AS commands
            FROM candidates WHERE project_id=$1 AND (id=(SELECT id FROM preferred) OR id=$3)
            ORDER BY (id=(SELECT id FROM preferred)) DESC
        "#,
        )
        .bind(uuid(project))
        .bind(kind == CandidateEntryKind::Panel)
        .bind(selected)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        rows.iter().map(summary).collect()
    }
    pub(super) async fn db_put_candidate(&mut self, c: &Candidate) -> StoreResult<()> {
        // Baseline is immutable. Checkpoints replace only the proposed projection.
        sqlx::query("INSERT INTO candidates (id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,execution_input,operations) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) ON CONFLICT (id) DO UPDATE SET state=EXCLUDED.state,updated_at=EXCLUDED.updated_at,view=EXCLUDED.view,batches=EXCLUDED.batches,progress=EXCLUDED.progress,applied_rounds=EXCLUDED.applied_rounds,reviewed_by=EXCLUDED.reviewed_by,execution_input=EXCLUDED.execution_input,operations=EXCLUDED.operations")
            .bind(c.id).bind(uuid(c.project_id)).bind(c.target.map(uuid)).bind(text(c.state))
            .bind(c.created_at).bind(c.updated_at).bind(&c.base_hash).bind(json(&c.base)?).bind(json(&c.view)?)
            .bind(json(&c.batches)?).bind(&c.progress).bind(c.parent).bind(json(&c.applied_rounds)?).bind(c.reviewed_by.map(uuid)).bind(&c.execution_input).bind(json(&c.operations)?)
            .execute(&mut *self.tx).await.map(|_| ()).map_err(unavailable)
    }
}
