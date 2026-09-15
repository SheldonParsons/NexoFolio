use crate::capture_store::{PostgresCaptureStore, err};
use nexofolio_contracts::{ProjectId, Result};
use sqlx::Row;
impl PostgresCaptureStore {
    /// Published/candidate/sample references pin payloads. Legacy ingestion records are untouched.
    pub async fn collect_expired_evidence(&self) -> Result<u64> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        sqlx::query("SET LOCAL lock_timeout='1s'")
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let acquired: bool = sqlx::query_scalar(
            "SELECT pg_try_advisory_xact_lock(hashtextextended('evidence-retention',0))",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
        if !acquired {
            return Ok(0);
        }
        // Bound both work and lock footprint; pinned rows with no temporary index do not starve later work.
        let pending = sqlx::query(
            "SELECT e.id,e.project_id FROM capture_events e
             WHERE e.received_at<clock_timestamp()-interval '24 hours' AND e.evidence_status='completed'
             AND (EXISTS(SELECT 1 FROM evidence_value_index v WHERE v.event_id=e.id)
               OR (e.raw_hash IS NOT NULL
                 AND NOT EXISTS(SELECT 1 FROM evidence_samples s WHERE s.event_id=e.id)
                 AND NOT EXISTS(SELECT 1 FROM evidence_pins p WHERE p.event_id=e.id)))
             ORDER BY e.received_at,e.id LIMIT 500",
        ).fetch_all(&mut *tx).await.map_err(err)?;
        let events: Vec<uuid::Uuid> = pending.iter().map(|r| r.get("id")).collect();
        let projects: std::collections::BTreeSet<uuid::Uuid> =
            pending.iter().map(|r| r.get("project_id")).collect();
        for project in projects {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("evidence-project:{project}"))
                .execute(&mut *tx)
                .await
                .map_err(err)?;
        }
        sqlx::query("DELETE FROM evidence_value_index WHERE event_id=ANY($1)")
            .bind(&events)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let expired = sqlx::query(
            "UPDATE capture_events e SET raw_hash=NULL WHERE e.id=ANY($1)
             AND raw_hash IS NOT NULL AND evidence_status='completed'
             AND NOT EXISTS(SELECT 1 FROM evidence_samples s WHERE s.event_id=e.id)
             AND NOT EXISTS(SELECT 1 FROM evidence_pins p WHERE p.event_id=e.id) RETURNING ingestion_id",
        ).bind(&events).fetch_all(&mut *tx).await.map_err(err)?;
        let ids: Vec<uuid::Uuid> = expired
            .iter()
            .filter_map(|r| r.get::<Option<uuid::Uuid>, _>("ingestion_id"))
            .collect();
        if !ids.is_empty() {
            sqlx::query(
                r#"UPDATE ingestion_inbox i
            SET raw_record=NULL
            WHERE i.id=ANY($1)
            AND i.status='completed'
            AND NOT EXISTS(SELECT 1
            FROM interface_observed_revisions r
            WHERE r.origin_ingestion_id=i.id)
            AND NOT EXISTS(SELECT 1
            FROM interface_observed_differences d
            WHERE d.origin_ingestion_id=i.id)
            AND NOT EXISTS(SELECT 1
            FROM capture_events e
            WHERE e.ingestion_id=i.id
            AND (e.raw_hash IS NOT NULL
            OR e.evidence_status<>'completed'))"#,
            )
            .bind(ids)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
        // Delete database references only when no event/asset refers to the object; filesystem deletion
        // occurs under the same content lock used by writers and is retriable on failure.
        let orphans = sqlx::query(
            r#"SELECT b.project_id,b.sha256
            FROM capture_blobs b
            WHERE b.created_at<clock_timestamp()-interval '24 hours'
            AND NOT EXISTS(SELECT 1
            FROM capture_events e
            WHERE e.project_id=b.project_id
            AND e.raw_hash=b.sha256)
            AND NOT EXISTS(SELECT 1
            FROM capture_assets a
            WHERE a.project_id=b.project_id
            AND a.sha256=b.sha256)
            LIMIT 100"#,
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(err)?;
        for row in orphans {
            let project: ProjectId = row
                .get::<uuid::Uuid, _>("project_id")
                .to_string()
                .parse()
                .unwrap();
            let hash: String = row.get("sha256");
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("blob:{project}:{hash}"))
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            // FK checks prevent removing newly referenced objects. Keep files until DB delete succeeds.
            let deleted = sqlx::query(
                r#"DELETE
            FROM capture_blobs b
            WHERE project_id=$1
            AND sha256=$2
            AND NOT EXISTS(SELECT 1
            FROM capture_events e
            WHERE e.project_id=b.project_id
            AND e.raw_hash=b.sha256)
            AND NOT EXISTS(SELECT 1
            FROM capture_assets a
            WHERE a.project_id=b.project_id
            AND a.sha256=b.sha256)"#,
            )
            .bind(row.get::<uuid::Uuid, _>("project_id"))
            .bind(&hash)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
            if deleted.rows_affected() == 1 {
                self.blobs.delete(project, &hash).await?;
            }
        }
        tx.commit().await.map_err(err)?;
        Ok(expired.len() as u64)
    }
}
