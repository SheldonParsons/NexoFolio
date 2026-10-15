//! Bounded reference proof inside the existing project transaction, never a cached preview.
use super::{
    codec::{unavailable, uuid},
    retention::{RetentionReport, input_refs},
    transaction::PostgresTx,
};
use nexofolio_common::ProjectId;
use nexofolio_knowledge_contracts::StoreResult;
use serde_json::Value;
use sqlx::Row;
use std::{collections::HashSet, time::Instant};

impl PostgresTx {
    pub(super) async fn retention_references(
        &mut self,
        project: ProjectId,
        eligible: &[uuid::Uuid],
        report: &mut RetentionReport,
        started: Instant,
    ) -> StoreResult<Option<HashSet<uuid::Uuid>>> {
        if eligible.is_empty() {
            return Ok(Some(HashSet::new()));
        }
        sqlx::query("SAVEPOINT retention_reference_check")
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        // Reference queries have their own shorter budget, leaving time to commit
        // compaction already completed under the same project lock.
        sqlx::query("SET LOCAL statement_timeout='500ms'")
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        let checked = self
            .check_retention_references(project, eligible, report, started)
            .await;
        if checked.is_err() {
            sqlx::query("ROLLBACK TO SAVEPOINT retention_reference_check")
                .execute(&mut *self.tx)
                .await
                .map_err(unavailable)?;
        }
        sqlx::query("RELEASE SAVEPOINT retention_reference_check")
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("SET LOCAL statement_timeout='1500ms'")
            .execute(&mut *self.tx)
            .await
            .map_err(unavailable)?;
        match checked {
            Ok(refs) => Ok(Some(refs)),
            Err(reason) => {
                report.skip(reason);
                // No partial proof may accrue a continuous no-reference grace.
                sqlx::query("UPDATE candidates SET archive_unreferenced_since=NULL WHERE project_id=$1 AND id=ANY($2) AND archive_unreferenced_since IS NOT NULL")
                    .bind(uuid(project)).bind(eligible).execute(&mut *self.tx).await.map_err(unavailable)?;
                Ok(None)
            }
        }
    }

    async fn check_retention_references(
        &mut self,
        project: ProjectId,
        eligible: &[uuid::Uuid],
        report: &mut RetentionReport,
        started: Instant,
    ) -> Result<HashSet<uuid::Uuid>, &'static str> {
        if started.elapsed().as_secs() >= 3 {
            return Err("reference_time_budget");
        }
        let unknown: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidates WHERE project_id=$1 AND execution_input IS NOT NULL AND coalesce(execution_input->>'version','unknown') NOT IN ('2','3'))")
            .bind(uuid(project)).fetch_one(&mut *self.tx).await.map_err(|_|"reference_query_incomplete")?;
        if unknown {
            return Err("unknown_input_format");
        }
        let unknown: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidates WHERE project_id=$1 AND history_pruned_at IS NULL AND (jsonb_typeof(base->'fields') IS DISTINCT FROM 'array' OR jsonb_typeof(view->'fields') IS DISTINCT FROM 'array' OR jsonb_typeof(batches) IS DISTINCT FROM 'array'))")
            .bind(uuid(project)).fetch_one(&mut *self.tx).await.map_err(|_|"reference_query_incomplete")?;
        if unknown {
            return Err("unknown_history_shape");
        }
        // Only references to this batch can be returned (at most 20 IDs). An
        // unrelated row count or body size cannot block material release above.
        let mut refs: HashSet<uuid::Uuid> = sqlx::query_scalar(include_str!("retention_refs.sql"))
            .bind(uuid(project))
            .bind(eligible)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(|_| "reference_query_incomplete")?
            .into_iter()
            .collect();
        report.reference_records += refs.len() as i64;
        let mut cursor = (uuid::Uuid::nil(), uuid::Uuid::nil(), String::new());
        loop {
            if started.elapsed().as_secs() >= 3 {
                return Err("reference_time_budget");
            }
            // Keyset pages contain only identities/lengths. Read one field-input
            // block at a time, including unpublished attempts. Never load docs.
            let page=sqlx::query("SELECT f.candidate_id,f.snapshot_id,f.sha256,octet_length(f.content)::bigint AS bytes FROM frozen_input_chunks f JOIN candidates c ON c.project_id=f.project_id AND c.id=f.candidate_id WHERE f.project_id=$1 AND c.execution_released_at IS NULL AND (f.candidate_id,f.snapshot_id,f.sha256)>($2,$3,$4) AND ltrim(f.content) LIKE '[%' ORDER BY f.candidate_id,f.snapshot_id,f.sha256 LIMIT 64")
                .bind(uuid(project)).bind(cursor.0).bind(cursor.1).bind(&cursor.2).fetch_all(&mut *self.tx).await.map_err(|_|"reference_query_incomplete")?;
            if page.is_empty() {
                return Ok(refs);
            }
            for row in page {
                cursor = (
                    row.get("candidate_id"),
                    row.get("snapshot_id"),
                    row.get("sha256"),
                );
                let bytes: i64 = row.get("bytes");
                if bytes > 16 * 1024 * 1024
                    || report.reference_bytes + bytes > 128 * 1024 * 1024
                    || report.reference_records >= 20_000
                {
                    report
                        .deferred_objects
                        .insert(cursor.0, "frozen_reference_block_budget".into());
                    return Err("frozen_reference_block_budget");
                }
                if started.elapsed().as_secs() >= 3 {
                    return Err("reference_time_budget");
                }
                let content = self
                    .db_frozen_chunk(project, cursor.0, cursor.1, &cursor.2)
                    .await
                    .map_err(|_| "reference_query_incomplete")?
                    .ok_or("reference_block_missing")?;
                report.reference_bytes += bytes;
                report.reference_records += 1;
                let fields: Vec<Value> =
                    serde_json::from_str(&content).map_err(|_| "reference_block_invalid")?;
                refs.extend(
                    input_refs(&fields)
                        .map_err(|_| "reference_block_invalid")?
                        .into_iter()
                        .filter(|id| eligible.contains(id)),
                );
            }
        }
    }
}
