mod facts;
mod links;
mod ui;
use crate::capture_store::{PostgresCaptureStore, authorize, err, uuid};
use facts::save_fact;
use links::link_event;
use nexofolio_contracts::*;
use nexofolio_evidence::{FactDraft, ValueOccurrence};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, Transaction};
use ui::*;
use uuid::Uuid;
fn hash(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("serializes"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn match_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(_) => v.to_string(),
        _ => String::new(),
    }
}
fn field_key(f: &EvidenceFieldRef) -> String {
    hash(&serde_json::to_value(f).unwrap())
}
struct Event {
    id: Uuid,
    project: ProjectId,
    env: EnvironmentId,
    producer: Uuid,
    actor: Uuid,
    kind: CaptureKind,
    context: Option<CaptureContext>,
    ingestion: Option<Uuid>,
    raw_hash: String,
    generation: i64,
    at: chrono::DateTime<chrono::Utc>,
}
impl Event {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            id: row.get("id"),
            project: row
                .get::<Uuid, _>("project_id")
                .to_string()
                .parse()
                .unwrap(),
            env: row
                .get::<Uuid, _>("environment_id")
                .to_string()
                .parse()
                .unwrap(),
            producer: row.get("producer_id"),
            actor: row.get("actor_id"),
            kind: serde_json::from_value(json!(row.get::<String, _>("kind"))).map_err(|_| {
                Error::Unavailable {
                    component: "capture_kind",
                }
            })?,
            context: row
                .get::<Option<Value>, _>("context")
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::Unavailable {
                    component: "capture_context",
                })?,
            ingestion: row.get("ingestion_id"),
            raw_hash: row
                .get::<Option<String>, _>("raw_hash")
                .ok_or(Error::Unavailable {
                    component: "capture_payload",
                })?,
            generation: row.get("generation"),
            at: row.get("captured_at"),
        })
    }
}
impl PostgresCaptureStore {
    /// Offline upgrade of legacy derived evidence. Original facts remain addressable;
    /// a retry resumes pending events instead of counting completed events twice.
    pub async fn refresh_legacy_evidence(&self, project: ProjectId) -> Result<u64> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("evidence-project:{project}"))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let legacy:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE project_id=$1 AND data->>'evidence_rule_version'='legacy' AND data->>'superseded' IS DISTINCT FROM 'true'").bind(uuid(project)).fetch_one(&mut *tx).await.map_err(err)?;
        if legacy > 0 {
            let unsafe_state: bool = sqlx::query_scalar(
                r#"SELECT
              EXISTS(SELECT 1 FROM evidence_facts WHERE project_id=$1
                AND data->>'superseded' IS DISTINCT FROM 'true'
                AND data->>'evidence_rule_version' IS DISTINCT FROM 'legacy')
              OR EXISTS(SELECT 1 FROM capture_events WHERE project_id=$1
                AND (raw_hash IS NULL OR evidence_status<>'completed'))"#,
            )
            .bind(uuid(project))
            .fetch_one(&mut *tx)
            .await
            .map_err(err)?;
            if unsafe_state {
                return Err(Error::invalid(
                    "LEGACY_REFRESH_REQUIRES_COMPLETE_RAW_AND_OFFLINE_LEGACY_STATE",
                ));
            }
            // Only disposable correlation indexes are rebuilt. Facts, samples, pins,
            // receipts, original definitions and recorded payloads remain intact.
            for table in [
                "evidence_relation_pairs",
                "evidence_relation_values",
                "evidence_ui_pairs",
            ] {
                sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE fact_id IN (SELECT id FROM evidence_facts WHERE project_id=$1)")))
                    .bind(uuid(project)).execute(&mut *tx).await.map_err(err)?;
            }
            for table in ["evidence_value_index", "evidence_ui_bindings"] {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DELETE FROM {table} WHERE project_id=$1"
                )))
                .bind(uuid(project))
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            }
            sqlx::query("UPDATE evidence_facts SET data=data||'{\"superseded\":true}'::jsonb,key_hash='superseded:'||id::text WHERE project_id=$1 AND data->>'evidence_rule_version'='legacy'")
                .bind(uuid(project)).execute(&mut *tx).await.map_err(err)?;
            sqlx::query("UPDATE capture_events SET evidence_status='pending',evidence_coverage=NULL,attempts=0,retry_at=clock_timestamp(),lease_until=NULL,generation=generation+1,error_code=NULL WHERE project_id=$1")
                .bind(uuid(project)).execute(&mut *tx).await.map_err(err)?;
            sqlx::query("UPDATE capture_backlog SET pending=(SELECT count(*) FROM capture_events WHERE project_id=$1 AND evidence_status<>'completed') WHERE project_id=$1")
                .bind(uuid(project)).execute(&mut *tx).await.map_err(err)?;
        }
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM capture_events WHERE project_id=$1 AND evidence_status<>'completed'")
            .bind(uuid(project)).fetch_one(&mut *tx).await.map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(pending as u64)
    }
    pub async fn process_evidence_one(&self) -> Result<bool> {
        let row=sqlx::query(r#"WITH picked AS (SELECT id
            FROM capture_events
            WHERE (evidence_status='pending'
            AND retry_at<=clock_timestamp())
            OR (evidence_status='processing'
            AND lease_until<clock_timestamp())
            ORDER BY received_at,id FOR UPDATE SKIP LOCKED
            LIMIT 1) UPDATE capture_events e
            SET evidence_status='processing',generation=generation+1,attempts=attempts+1,lease_until=clock_timestamp()+interval '120 seconds'
            FROM picked p
            WHERE e.id=p.id
            RETURNING e.*"#).fetch_optional(&self.database.pool).await.map_err(err)?;
        let Some(row) = row else {
            return Ok(false);
        };
        let event = Event::from_row(&row)?;
        let result = self.extract_event(&event).await;
        if let Err(error) = result {
            let waiting = matches!(error, Error::NotConfigured { .. });
            sqlx::query(r#"UPDATE capture_events
            SET evidence_status=CASE WHEN $3
            OR attempts<5 THEN 'pending' ELSE 'failed' END,retry_at=clock_timestamp()+interval '2 seconds',lease_until=NULL,error_code=$4
            WHERE id=$1
            AND generation=$2
            AND evidence_status='processing'"#).bind(event.id).bind(event.generation).bind(waiting).bind(if waiting{"WAITING_DEPENDENCY"}else if matches!(error,Error::Unavailable{component:"evidence_relation_budget"}) {"RELATION_SEARCH_LIMIT"} else {"EVIDENCE_PROCESSING_FAILED"}).execute(&self.database.pool).await.map_err(err)?;
            if !waiting {
                return Err(error);
            }
        }
        Ok(true)
    }
    async fn extract_event(&self, event: &Event) -> Result<()> {
        let payload: Value =
            serde_json::from_slice(&self.blobs.get(event.project, &event.raw_hash).await?)
                .map_err(|_| Error::Unavailable {
                    component: "capture_payload",
                })?;
        let mut facts = Vec::new();
        let mut values = Vec::new();
        let mut coverage = json!({"request_complete":false,"response_complete":false});
        if let Some(ingestion) = event.ingestion {
            let binding=sqlx::query("SELECT o.interface_id,o.compared_revision_id,i.path_identity,r.definition FROM interface_observations o JOIN ingestion_inbox i ON i.id=o.ingestion_id JOIN interface_observed_revisions r ON r.id=o.compared_revision_id WHERE o.ingestion_id=$1").bind(ingestion).fetch_optional(&self.database.pool).await.map_err(err)?.ok_or(Error::NotConfigured{capability:"parent_observation"})?;
            let reference = FieldRef {
                interface_id: binding
                    .get::<Uuid, _>("interface_id")
                    .to_string()
                    .parse()
                    .unwrap(),
                environment_id: event.env,
                revision_id: binding
                    .get::<Uuid, _>("compared_revision_id")
                    .to_string()
                    .parse()
                    .unwrap(),
                location: String::new(),
                path: String::new(),
            };
            let path = binding
                .get::<Option<Value>, _>("path_identity")
                .map(serde_json::from_value::<PathIdentity>)
                .transpose()
                .map_err(|_| Error::Unavailable {
                    component: "path_identity",
                })?;
            let mut extraction =
                nexofolio_evidence::extract_http(&payload, reference, path.as_ref());
            extraction.bind_source(
                event.project,
                ingestion,
                &binding.get::<Value, _>("definition"),
            );
            coverage = json!({"request_complete":extraction.request_complete,"response_complete":extraction.response_complete});
            facts = extraction.facts;
            values = extraction.values;
            facts.extend(nexofolio_evidence::dictionary_facts(&payload, &values));
            if !extraction.limitations.is_empty() {
                facts.push(FactDraft {
                    kind: "extraction_limit".into(),
                    subject: json!({"ingestion_id":ingestion}),
                    data: json!({"limitations":extraction.limitations}),
                });
            }
        } else {
            facts.extend(nexofolio_evidence::ui_facts(
                &event.kind,
                &payload,
                &event.context,
            ));
        }
        if matches!(event.kind, CaptureKind::ImageReference) {
            let asset: ImageReferencePayload = serde_json::from_value(payload.clone())
                .map_err(|_| Error::invalid("invalid image reference"))?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM capture_assets WHERE project_id=$1 AND id=$2)",
            )
            .bind(uuid(event.project))
            .bind(asset.asset_id)
            .fetch_one(&self.database.pool)
            .await
            .map_err(err)?;
            if !exists {
                return Err(Error::NotConfigured {
                    capability: "image_asset",
                });
            }
        }
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM capture_events WHERE id=$1 AND generation=$2 AND evidence_status='processing' AND lease_until>clock_timestamp())").bind(event.id).bind(event.generation).fetch_one(&mut *tx).await.map_err(err)?;
        if !valid {
            return Err(Error::Conflict);
        }
        // Serialize only evidence within this project/context; models never run inside this transaction.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("evidence-project:{}", event.project))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        sqlx::query("UPDATE capture_events SET evidence_coverage=$2 WHERE id=$1")
            .bind(event.id)
            .bind(&coverage)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        facts::retain_values(&mut tx, event, &mut facts).await?;
        let mut unique = std::collections::HashSet::new();
        for mut fact in facts {
            if fact.kind == "dictionary_mapping_candidate" {
                let conditions = fact.data["scope"].take();
                fact.data["scope"] = json!({"request_hash":hash(&conditions),"actor_id":event.actor,"complete":conditions["complete"]});
            }
            let key = hash(&json!([fact.kind, fact.subject, fact.data]));
            if unique.insert(key) {
                save_fact(&mut tx, event, &fact, None).await?;
            }
        }
        if let Some(ctx) = &event.context {
            if let (Some(start), Some(end)) =
                (ctx.request_started_at_ms, ctx.response_completed_at_ms)
                && end >= start
            {
                let mut indexed = std::collections::HashSet::new();
                for value in &values {
                    if indexed.insert((field_key(&value.field), value.pointer.clone())) {
                        sqlx::query(r#"INSERT INTO evidence_value_index(event_id,project_id,environment_id,producer_id,page_instance_id,frame_instance_id,view_id,interaction_id,field_key,field_ref,pointer,value,match_key,direction,started_at,available_at,actor_id,browser_instance_id)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
            ON CONFLICT DO NOTHING"#)
      .bind(event.id).bind(uuid(event.project)).bind(uuid(event.env)).bind(event.producer).bind(ctx.page_instance_id).bind(ctx.frame_instance_id).bind(ctx.view_id).bind(ctx.interaction_id).bind(field_key(&value.field)).bind(serde_json::to_value(&value.field).unwrap()).bind(&value.pointer).bind(&value.value).bind(if nexofolio_evidence::informative(&value.value) && nexofolio_evidence::relation_field(&value.field) {match_key(&value.value)} else {String::new()}).bind(value.direction).bind(start).bind(end).bind(event.actor).bind(ctx.browser_instance_id).execute(&mut *tx).await.map_err(err)?;
                    }
                }
                link_event(&mut tx, event).await?;
                correlate_ui(
                    &mut tx,
                    event,
                    ctx,
                    &values,
                    start,
                    &payload,
                    self.blobs.as_ref(),
                )
                .await?;
            }
            if matches!(event.kind, CaptureKind::Interaction) {
                links::late_interaction(&mut tx, event, ctx).await?;
            }
            if matches!(
                event.kind,
                CaptureKind::Interaction | CaptureKind::UiSnapshot
            ) {
                link_late_ui(&mut tx, event, ctx, &payload, self.blobs.as_ref()).await?;
            }
        }
        let updated=sqlx::query(r#"UPDATE capture_events
            SET evidence_status='completed',processed_at=clock_timestamp(),lease_until=NULL,error_code=NULL
            WHERE id=$1
            AND generation=$2
            AND evidence_status='processing'
            AND lease_until>clock_timestamp()"#).bind(event.id).bind(event.generation).execute(&mut *tx).await.map_err(err)?.rows_affected();
        if updated != 1 {
            return Err(Error::Conflict);
        }
        sqlx::query(
            "UPDATE capture_backlog SET pending=pending-1 WHERE project_id=$1 AND pending>0",
        )
        .bind(uuid(event.project))
        .execute(&mut *tx)
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(())
    }
    pub async fn retry_evidence(&self, user: UserId, project: ProjectId, id: Uuid) -> Result<()> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        authorize(&mut tx, user, project).await?;
        let n=sqlx::query(r#"UPDATE capture_events
            SET evidence_status='pending',attempts=0,generation=generation+1,retry_at=clock_timestamp(),error_code=NULL
            WHERE id=$1
            AND project_id=$2
            AND evidence_status='failed'"#).bind(id).bind(uuid(project)).execute(&mut *tx).await.map_err(err)?.rows_affected();
        if n != 1 {
            return Err(Error::Conflict);
        }
        tx.commit().await.map_err(err)?;
        Ok(())
    }
}
