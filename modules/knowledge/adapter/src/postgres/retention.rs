//! One explicitly requested project batch. Ordinary API reads never enter here.
use super::{
    PROJECT_LOCK, PostgresKnowledge,
    codec::{unavailable, uuid},
    transaction::PostgresTx,
};
use chrono::{DateTime, Duration, Utc};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{FrozenManifest, frozen_input_hash};
use nexofolio_knowledge_contracts::{StoreError, StoreResult};
use serde_json::Value;
use sqlx::Row;
use std::collections::HashSet;

pub use nexofolio_knowledge_contracts::retention::RetentionReport;

impl PostgresKnowledge {
    /// Caller supplies a clock for deterministic testing; admin always uses wall time.
    /// Cancellation before commit rolls back. A lost commit acknowledgement is
    /// indeterminate; persisted markers make the next batch safe to repeat.
    pub async fn retain_history(
        &self,
        project: ProjectId,
        now: DateTime<Utc>,
        after: Option<uuid::Uuid>,
        limit: usize,
        execute: bool,
    ) -> StoreResult<RetentionReport> {
        if !(1..=20).contains(&limit) {
            return Err(StoreError::Unavailable);
        }
        let mut tx = PostgresTx {
            apply_buffer: None,
            tx: self.pool.begin().await.map_err(unavailable)?,
        };
        sqlx::query("SET LOCAL statement_timeout='1500ms'")
            .execute(&mut *tx.tx)
            .await
            .map_err(unavailable)?;
        let mut report = RetentionReport::default();
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(project.to_string())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(unavailable)?;
        if !locked {
            report.skip("project_busy");
            return Ok(report);
        }
        let enabled: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT enabled_at FROM retention_projects WHERE project_id=$1")
                .bind(uuid(project))
                .fetch_optional(&mut *tx.tx)
                .await
                .map_err(unavailable)?;
        if enabled.is_none() && execute {
            sqlx::query("INSERT INTO retention_projects(project_id,enabled_at) VALUES($1,$2)")
                .bind(uuid(project))
                .bind(now)
                .execute(&mut *tx.tx)
                .await
                .map_err(unavailable)?;
        }
        if enabled.is_none_or(|at| at > now - Duration::days(7)) {
            report.skip("activation_grace_7_days");
        } else {
            tx.retention_batch(project, now, after, limit, &mut report)
                .await?;
        }
        if execute {
            tx.tx.commit().await.map_err(unavailable)?;
            report.committed = true;
        }
        Ok(report)
    }
}

impl PostgresTx {
    async fn retention_batch(
        &mut self,
        project: ProjectId,
        now: DateTime<Utc>,
        after: Option<uuid::Uuid>,
        limit: usize,
        report: &mut RetentionReport,
    ) -> StoreResult<()> {
        // Only selected headers cross the wire. Legacy v2 payloads are never
        // loaded just to release them; large evidence bodies stay in their blocks.
        let rows = sqlx::query(include_str!("retention_batch.sql"))
            .bind(uuid(project))
            .bind(after)
            .bind(limit as i64)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        let recent: HashSet<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM candidates WHERE project_id=$1 AND state IN ('applied','discarded') ORDER BY updated_at DESC,id DESC LIMIT 5")
            .bind(uuid(project)).fetch_all(&mut *self.tx).await.map_err(unavailable)?.into_iter().collect();
        let started = std::time::Instant::now();
        let mut eligible = Vec::new();
        for row in rows {
            if started.elapsed().as_secs() >= 3 {
                report.skip("batch_time_budget");
                break;
            }
            let id: uuid::Uuid = row.get("id");
            report.next = Some(id);
            report.scanned += 1;
            let protected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM curation_tasks WHERE project_id=$1 AND (candidate_id=$2 OR parent=$2) AND state IN ('queued','running','waiting','needs_attention')) OR EXISTS(SELECT 1 FROM candidates WHERE project_id=$1 AND parent=$2 AND state NOT IN ('applied','discarded'))")
                .bind(uuid(project)).bind(id).fetch_one(&mut *self.tx).await.map_err(unavailable)?;
            if !matches!(row.get::<&str, _>("state"), "applied" | "discarded") || protected {
                report.skip("recoverable_or_active");
                continue;
            }
            let ended: DateTime<Utc> = row.get("updated_at");
            if ended > now - Duration::days(7) {
                report.skip("closed_less_than_7_days");
                continue;
            }
            if !row.get::<bool, _>("history_shape_valid") {
                report.skip("unknown_history_shape");
                report
                    .deferred_objects
                    .insert(id, "unknown_history_shape".into());
                continue;
            }
            if row.get::<bool, _>("header_oversized") {
                report.skip("header_over_4_mib");
                report
                    .deferred_objects
                    .insert(id, "header_over_4_mib".into());
                continue;
            }
            let input: Option<Value> = row.get("execution_input");
            if input
                .as_ref()
                .is_some_and(|v| v["version"] != 2 && v["version"] != 3)
            {
                report.skip("unknown_input_format");
                report
                    .deferred_objects
                    .insert(id, "unknown_input_format".into());
                continue;
            }
            let header = input
                .as_ref()
                .or(row.get::<Option<Value>, _>("evidence_input").as_ref())
                .cloned();
            let manifest = match header.as_ref().filter(|v| v["version"] == 3) {
                Some(v) => {
                    let Ok(m) = serde_json::from_value::<FrozenManifest>(v["manifest"].clone())
                    else {
                        report.skip("invalid_manifest");
                        continue;
                    };
                    if m.version != 1
                        || m.project != project
                        || m.candidate != id
                        || v["manifest_hash"].as_str() != Some(frozen_input_hash(v).as_str())
                    {
                        report.skip("invalid_manifest");
                        continue;
                    }
                    Some(m)
                }
                None => None,
            };
            if row
                .get::<Option<DateTime<Utc>>, _>("execution_released_at")
                .is_none()
                && let Some(m) = &manifest
            {
                // The published header is verified; preserve all of its evidence
                // identities without loading immutable document bodies again.
                let keep: HashSet<_> = m
                    .endpoints
                    .values()
                    .flat_map(|e| std::iter::once(&e.values).chain(e.documents.values()))
                    .map(|r| r.sha256.clone())
                    .collect();
                let present: i64=sqlx::query_scalar("SELECT count(*) FROM frozen_input_chunks WHERE project_id=$1 AND candidate_id=$2 AND snapshot_id=$3 AND sha256=ANY($4)")
                    .bind(uuid(project)).bind(id).bind(m.snapshot).bind(keep.iter().collect::<Vec<_>>())
                    .fetch_one(&mut *self.tx).await.map_err(unavailable)?;
                if present as usize != keep.len() {
                    report.skip("invalid_evidence_chunks");
                    continue;
                }
            }
            // Applied rounds form one rollback group; never split it. Independent
            // manual rounds and revisions stay intact, including their references.
            if row
                .get::<Option<DateTime<Utc>>, _>("history_pruned_at")
                .is_none()
                && ended < now - Duration::days(30)
                && !recent.contains(&id)
            {
                let rounds: Vec<uuid::Uuid> =
                    serde_json::from_value(row.get("applied_rounds")).map_err(unavailable)?;
                let unsafe_round: bool = sqlx::query_scalar("SELECT (SELECT count(*) FROM rounds WHERE project_id=$1 AND id=ANY($2)) <> cardinality($2) OR EXISTS(SELECT 1 FROM rounds WHERE project_id=$1 AND id=ANY($2) AND (state='running' OR finished_at >= $3 OR coalesce(author->>'by','unknown')<>'curate')) OR EXISTS(SELECT 1 FROM candidates c, LATERAL jsonb_array_elements_text(c.applied_rounds) r WHERE c.project_id=$1 AND c.id<>$4 AND c.history_pruned_at IS NULL AND r::uuid=ANY($2))")
                    .bind(uuid(project)).bind(&rounds).bind(now-Duration::days(30)).bind(id)
                    .fetch_one(&mut *self.tx).await.map_err(unavailable)?;
                if unsafe_round {
                    report.skip("retained_rollback_group");
                } else {
                    sqlx::query("UPDATE rounds SET history_pruned_at=$3,summary=coalesce(summary,'') || '；历史已精简，仅保留摘要，完整对比与回滚已到期' WHERE project_id=$1 AND id=ANY($2)")
                        .bind(uuid(project)).bind(&rounds).bind(now).execute(&mut *self.tx).await.map_err(unavailable)?;
                    sqlx::query("DELETE FROM revisions v USING rounds r WHERE v.round_id=r.id AND r.project_id=$1 AND r.id=ANY($2)")
                        .bind(uuid(project)).bind(&rounds)
                        .execute(&mut *self.tx)
                        .await
                        .map_err(unavailable)?;
                    sqlx::query("UPDATE candidates SET history_pruned_at=$3,progress=progress || jsonb_build_object('history_available',false,'history_message','历史已精简，仅保留摘要；详情、对比与回滚已到期','retained_batches',jsonb_array_length(batches),'retained_commands',(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0) FROM jsonb_array_elements(batches) b),'lifecycle_version',coalesce(base->'lifecycle'->'version','0'::jsonb)),base='{}',view='{}',batches='[]' WHERE project_id=$1 AND id=$2")
                        .bind(uuid(project)).bind(id).bind(now).execute(&mut *self.tx).await.map_err(unavailable)?;
                    report.compacted += 1;
                }
            }
            if row
                .get::<Option<DateTime<Utc>>, _>("execution_released_at")
                .is_none()
            {
                // Preserve the original, hash-verified v3 header for value reads.
                // Only values+documents survive; execution facts/inputs/links do not.
                let keep: Vec<String> = manifest
                    .as_ref()
                    .map(|m| {
                        m.endpoints
                            .values()
                            .flat_map(|e| std::iter::once(&e.values).chain(e.documents.values()))
                            .map(|r| r.sha256.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let released = input.is_some();
                let chunks = sqlx::query("DELETE FROM frozen_input_chunks WHERE project_id=$1 AND candidate_id=$2 AND NOT(snapshot_id=$3 AND sha256=ANY($4))")
                    .bind(uuid(project)).bind(id).bind(manifest.as_ref().map(|m|m.snapshot).unwrap_or(uuid::Uuid::nil())).bind(&keep)
                    .execute(&mut *self.tx).await.map_err(unavailable)?.rows_affected();
                sqlx::query("UPDATE candidates SET evidence_input=$3,execution_input=NULL,execution_released_at=$4,progress=progress || '{\"execution_released\":true}'::jsonb WHERE project_id=$1 AND id=$2")
                    .bind(uuid(project)).bind(id).bind(input.filter(|v|v["version"]==3)).bind(now)
                    .execute(&mut *self.tx).await.map_err(unavailable)?;
                report.chunks_deleted += chunks;
                report.inputs_released += usize::from(released || chunks > 0);
            }
            eligible.push(id);
        }
        // A failed/bounded reference read cannot undo safe material compaction.
        // It never authorizes deletion: grace is reset until a complete check succeeds.
        let Some(refs) = self
            .retention_references(project, &eligible, report, started)
            .await?
        else {
            return Ok(());
        };
        for id in eligible {
            if started.elapsed().as_secs() >= 3 {
                report.skip("batch_time_budget");
                break;
            }
            if refs.contains(&id) {
                sqlx::query("UPDATE candidates SET archive_unreferenced_since=NULL WHERE project_id=$1 AND id=$2 AND archive_unreferenced_since IS NOT NULL")
                    .bind(uuid(project)).bind(id).execute(&mut *self.tx).await.map_err(unavailable)?;
                report.skip("archive_referenced");
                continue;
            }
            let row=sqlx::query("SELECT archive_unreferenced_since,archive_reclaimed_at FROM candidates WHERE project_id=$1 AND id=$2")
                .bind(uuid(project)).bind(id).fetch_one(&mut *self.tx).await.map_err(unavailable)?;
            if row
                .get::<Option<DateTime<Utc>>, _>("archive_reclaimed_at")
                .is_some()
            {
                report.skip("already_reclaimed");
                continue;
            }
            match row.get::<Option<DateTime<Utc>>, _>("archive_unreferenced_since") {
                None => {
                    sqlx::query("UPDATE candidates SET archive_unreferenced_since=$3 WHERE project_id=$1 AND id=$2")
                        .bind(uuid(project)).bind(id).bind(now).execute(&mut *self.tx).await.map_err(unavailable)?;
                    report.grace_started += 1;
                }
                Some(at) if at <= now - Duration::days(7) => {
                    let archives = sqlx::query(
                        "DELETE FROM field_value_archives WHERE project_id=$1 AND candidate_id=$2",
                    )
                    .bind(uuid(project))
                    .bind(id)
                    .execute(&mut *self.tx)
                    .await
                    .map_err(unavailable)?
                    .rows_affected();
                    let chunks = sqlx::query(
                        "DELETE FROM frozen_input_chunks WHERE project_id=$1 AND candidate_id=$2",
                    )
                    .bind(uuid(project))
                    .bind(id)
                    .execute(&mut *self.tx)
                    .await
                    .map_err(unavailable)?
                    .rows_affected();
                    sqlx::query("UPDATE candidates SET evidence_input=NULL,archive_reclaimed_at=$3,archive_unreferenced_since=NULL WHERE project_id=$1 AND id=$2")
                        .bind(uuid(project)).bind(id).bind(now).execute(&mut *self.tx).await.map_err(unavailable)?;
                    report.chunks_deleted += chunks;
                    if archives + chunks > 0 {
                        report.archives_reclaimed += 1;
                    } else {
                        report.skip("no_archive_payload");
                    }
                }
                Some(_) => report.skip("archive_grace_7_days"),
            }
        }
        Ok(())
    }
}

pub(super) fn input_refs(fields: &[Value]) -> StoreResult<Vec<uuid::Uuid>> {
    fields
        .iter()
        .filter_map(|f| f.get("observations"))
        .map(|v| {
            serde_json::from_value::<
                Option<nexofolio_contracts::field_values::FieldObservationSummary>,
            >(v.clone())
            .map(|s| s.and_then(|s| s.archive_id))
            .map_err(unavailable)
        })
        .collect::<StoreResult<Vec<_>>>()
        .map(|ids| ids.into_iter().flatten().collect())
}
