use super::{
    codec::{id, json, parse, text, unavailable, uuid},
    transaction::PostgresTx,
};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::knowledge::{Candidate, CandidateEntryKind, CandidateSummary, Command};
use nexofolio_knowledge_contracts::{CandidateAppend, CandidateAppendRound, StoreResult};
use sqlx::{Row, postgres::PgRow, types::Json};

fn candidate(row: &PgRow) -> StoreResult<Candidate> {
    if row
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("history_pruned_at")
        .ok()
        .flatten()
        .is_some()
    {
        return Err(nexofolio_knowledge_contracts::StoreError::HistoryExpired);
    }
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
    if !progress.is_object() {
        progress = serde_json::json!({});
    }
    if progress.get("history_available") != Some(&serde_json::json!(false)) {
        progress["lifecycle_version"] = row.get::<serde_json::Value, _>("lifecycle_version");
    }
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
                && progress
                    .get("recovery_blocked")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
        );
    }
    Ok(CandidateSummary {
        id: row.get("id"),
        project_id: id(row.get("project_id")),
        target: row.get::<Option<uuid::Uuid>, _>("target").map(id),
        state: parse(row.get("state"))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        commands: progress
            .get("retained_commands")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(row.get::<i64, _>("commands") as u64) as usize,
        progress,
    })
}
impl PostgresTx {
    pub(super) async fn db_candidate_application(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        // v2 validation has no execution-body dependency. v3 still carries the
        // complete signed header; all frozen references retain integrity checks.
        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,operations,history_pruned_at,CASE WHEN execution_input->>'version'='3' THEN execution_input END AS execution_input FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project)).bind(key).fetch_optional(&mut *self.tx).await.map_err(unavailable)?;
        row.as_ref().map(candidate).transpose()
    }
    pub(super) async fn db_put_candidate_decision(&mut self, c: &Candidate) -> StoreResult<()> {
        let result = sqlx::query("UPDATE candidates SET state=$3,updated_at=$4,applied_rounds=$5,reviewed_by=$6,operations=$7 WHERE project_id=$1 AND id=$2")
            .bind(uuid(c.project_id)).bind(c.id).bind(text(c.state)).bind(c.updated_at)
            .bind(Json(&c.applied_rounds)).bind(c.reviewed_by.map(uuid)).bind(Json(&c.operations))
            .execute(&mut *self.tx).await.map_err(unavailable)?;
        if result.rows_affected() != 1 {
            return Err(nexofolio_knowledge_contracts::StoreError::Unavailable);
        }
        Ok(())
    }

    pub(super) async fn db_candidate_append(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
        round: RoundId,
    ) -> StoreResult<Option<CandidateAppend>> {
        // JSONB extraction can still detoast base/batches, but old fields and commands
        // never cross the wire or enter the application's serde object graph.
        let row = sqlx::query(
            r#"
            SELECT state,progress->>'run_id' AS owner,base->'endpoints' AS endpoints,view,
                (SELECT coalesce(jsonb_agg(jsonb_build_object(
                    'id',b->'id','stored_id',b->>'id','author',b->'author','stored_author',(b->'author')::text,'state',b->'state',
                    'position',ord-1,'command_count',jsonb_array_length(b->'commands'))
                    ORDER BY ord),'[]'::jsonb)
                 FROM jsonb_array_elements(batches) WITH ORDINALITY AS r(b,ord)) AS rounds
            FROM candidates WHERE project_id=$1 AND id=$2
        "#,
        )
        .bind(uuid(project))
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        row.map(|r| {
            // Decode identities with the original serde contract, including UUID
            // spellings PostgreSQL's uuid input parser may not accept.
            let mut rounds = r
                .try_get::<Json<Vec<CandidateAppendRound>>, _>("rounds")
                .map_err(unavailable)?
                .0;
            rounds.retain(|r| r.id == round);
            Ok(CandidateAppend {
                project,
                id: key,
                state: parse(r.try_get("state").map_err(unavailable)?)?,
                owner: r
                    .try_get::<Option<String>, _>("owner")
                    .map_err(unavailable)?
                    .and_then(|s| s.parse().ok()),
                endpoints: r.try_get::<Json<_>, _>("endpoints").map_err(unavailable)?.0,
                view: r.try_get::<Json<_>, _>("view").map_err(unavailable)?.0,
                rounds,
            })
        })
        .transpose()
    }
    pub(super) async fn db_append_candidate_commands(
        &mut self,
        c: &CandidateAppend,
        round: &CandidateAppendRound,
        proposed: &[Command],
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<()> {
        // One UPDATE keeps view/history atomic. PostgreSQL may still rewrite all
        // batches JSONB/TOAST; this is not incremental physical storage.
        let result = sqlx::query(
            r#"
            UPDATE candidates SET updated_at=$3,view=$4,
                batches=jsonb_set(batches,ARRAY[$5::text,'commands'],
                    (batches->$5::int->'commands') || $6::jsonb,false)
            WHERE project_id=$1 AND id=$2 AND state='running'
                AND batches->$5::int->>'id'=$7
                AND batches->$5::int->>'state'='running'
                AND (batches->$5::int->'author')::text=$8
                AND jsonb_array_length(batches->$5::int->'commands')=$9
                AND (SELECT count(*) FROM jsonb_array_elements(batches) b WHERE b->>'id'=$7)=1
        "#,
        )
        .bind(uuid(c.project))
        .bind(c.id)
        .bind(at)
        .bind(Json(&c.view))
        .bind(round.position.to_string())
        .bind(Json(proposed))
        .bind(&round.stored_id)
        .bind(&round.stored_author)
        .bind(round.command_count as i64)
        .execute(&mut *self.tx)
        .await
        .map_err(unavailable)?;
        if result.rows_affected() != 1 {
            return Err(nexofolio_knowledge_contracts::StoreError::Unavailable);
        }
        Ok(())
    }
    pub(super) async fn db_recovery_candidates(
        &mut self,
        before: chrono::DateTime<chrono::Utc>,
        after: Option<uuid::Uuid>,
        limit: usize,
    ) -> StoreResult<Vec<nexofolio_contracts::knowledge::CandidateRecovery>> {
        let rows = sqlx::query("SELECT id,project_id FROM candidates WHERE state='running' AND updated_at<=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3")
            .bind(before)
            .bind(after)
            .bind(limit as i64)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        Ok(rows
            .iter()
            .map(|row| nexofolio_contracts::knowledge::CandidateRecovery {
                id: row.get("id"),
                project_id: id(row.get("project_id")),
            })
            .collect())
    }

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
        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,coalesce(base->'lifecycle'->'version','0'::jsonb) AS lifecycle_version,coalesce(execution_input->>'version' IN ('2','3'),false) AS input_frozen,progress,(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0)::bigint FROM jsonb_array_elements(batches) b) AS commands FROM candidates WHERE project_id=$1 AND state='running' ORDER BY created_at LIMIT 1")
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
        let started = std::time::Instant::now();
        let row = sqlx::query("SELECT * FROM candidates WHERE project_id=$1 AND id=$2")
            .bind(uuid(project))
            .bind(candidate_id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        let fetched = started.elapsed();
        let decoded = row.as_ref().map(candidate).transpose();
        tracing::info!(
            fetch_us = fetched.as_micros() as u64,
            decode_us = (started.elapsed() - fetched).as_micros() as u64,
            "candidate full read timing"
        );
        decoded
    }
    pub(super) async fn db_candidate_projection(
        &mut self,
        project: ProjectId,
        key: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        let row=sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,operations,history_pruned_at,NULL::jsonb AS execution_input FROM candidates WHERE project_id=$1 AND id=$2")
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
        let rows = sqlx::query("SELECT id,project_id,target,state,created_at,updated_at,coalesce(base->'lifecycle'->'version','0'::jsonb) AS lifecycle_version,coalesce(execution_input->>'version' IN ('2','3'),false) AS input_frozen,progress,(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0)::bigint FROM jsonb_array_elements(batches) b) AS commands FROM candidates WHERE project_id=$1 ORDER BY created_at DESC LIMIT $2")
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
            SELECT id,project_id,target,state,created_at,updated_at,coalesce(base->'lifecycle'->'version','0'::jsonb) AS lifecycle_version,coalesce(execution_input->>'version' IN ('2','3'),false) AS input_frozen,
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
