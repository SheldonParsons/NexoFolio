use crate::Postgres;
use async_trait::async_trait;
use nexofolio_contracts::{Error, Result};
use nexofolio_intake::{Admission, AdmissionResult, AdmissionStore, ReceiptStatus, RecordReceipt};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;

struct Head {
    hash: Option<Vec<u8>>,
    projection: Option<Value>,
    id: Uuid,
}
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresAdmission {
    database: Postgres,
}
impl PostgresAdmission {
    pub fn new(database: Postgres) -> Self {
        Self { database }
    }
}
fn db_error(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "ingestion",
    }
}
#[async_trait]
impl AdmissionStore for PostgresAdmission {
    async fn admit(&self, mut batch: Admission) -> Result<AdmissionResult> {
        let actor: Uuid = batch.actor.to_string().parse().expect("typed UUID");
        let project: Uuid = batch.project_id.to_string().parse().expect("typed UUID");
        let mut tx = self.database.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET LOCAL lock_timeout='3s'")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        // Hold the user's row against permission-sync replacement for this bounded batch.
        let user = sqlx::query(
            "SELECT enabled,grants_synced FROM users WHERE id=$1 AND instance=$2 FOR SHARE",
        )
        .bind(actor)
        .bind(&batch.instance)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or(Error::Unauthenticated)?;
        if !user.get::<bool, _>("enabled") {
            return Err(Error::Unauthenticated);
        }
        if !user.get::<bool, _>("grants_synced") {
            return Err(Error::Unavailable {
                component: "project_access",
            });
        }
        let project_row=sqlx::query("SELECT p.path_policy,EXISTS(SELECT 1 FROM user_project_access a WHERE a.user_id=$1 AND a.project_id=p.id) AS allowed FROM projects p WHERE p.id=$2 AND p.instance=$3 FOR SHARE OF p").bind(actor).bind(project).bind(&batch.instance).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or(Error::Forbidden)?;
        if !project_row.get::<bool, _>("allowed") {
            return Err(Error::Forbidden);
        }
        let policy: serde_json::Value = project_row.get("path_policy");
        let policy: nexofolio_contracts::PathPolicy =
            serde_json::from_value(policy).map_err(|_| Error::Unavailable {
                component: "path_policy",
            })?;
        if !policy.valid() {
            return Err(Error::Unavailable {
                component: "path_policy",
            });
        }
        for row in &mut batch.records {
            nexofolio_intake::apply_path_policy(row, &policy);
        }
        let environment =
            crate::environments::resolve(&mut tx, project, &batch.environment).await?;
        let environment_id: Uuid = environment.id.to_string().parse().expect("UUID");
        // Lock only involved idempotency records and interface/environment heads, ordered to avoid
        // reverse-order batches deadlocking. Hash collisions cause serialization, never false dedup.
        let mut locks = std::collections::BTreeSet::new();
        for row in &batch.records {
            locks.insert(format!(
                "record:{actor}:{}:{}",
                batch.source.instance_id, row.record_id
            ));
            locks.insert(format!(
                "head:{project}:{}:{}",
                environment_id, row.identity_key
            ));
        }
        let locks: Vec<_> = locks.into_iter().collect();
        sqlx::query("SELECT pg_advisory_xact_lock(k) FROM (SELECT DISTINCT hashtextextended(x,0) AS k FROM unnest($1::text[]) AS x ORDER BY k) AS ordered_locks").bind(locks).execute(&mut *tx).await.map_err(db_error)?;
        let ids: Vec<_> = batch.records.iter().map(|r| r.record_id).collect();
        let identities: Vec<_> = batch
            .records
            .iter()
            .map(|r| r.identity_key.clone())
            .collect();
        let previous=sqlx::query("SELECT record_id,content_hash,status,reason_code,ingestion_id FROM ingestion_receipts WHERE actor_id=$1 AND producer_id=$2 AND record_id=ANY($3)").bind(actor).bind(batch.source.instance_id).bind(&ids).fetch_all(&mut *tx).await.map_err(db_error)?;
        let mut known: std::collections::HashMap<Uuid, (Vec<u8>, String, String, Uuid)> = previous
            .iter()
            .map(|r| {
                (
                    r.get("record_id"),
                    (
                        r.get("content_hash"),
                        r.get("status"),
                        r.get("reason_code"),
                        r.get("ingestion_id"),
                    ),
                )
            })
            .collect();
        let previous=sqlx::query(r#"SELECT h.identity_key,h.structural_hash,h.algorithm_version,h.ingestion_id,h.structural_projection,CASE WHEN h.algorithm_version <> 'http-structure-2' THEN i.raw_record ELSE NULL END AS legacy_raw
            FROM ingestion_heads h
            JOIN ingestion_inbox i ON i.id=h.ingestion_id
            WHERE h.project_id=$1
            AND h.environment_id=$2
            AND h.identity_key=ANY($3)"#).bind(project).bind(environment_id).bind(&identities).fetch_all(&mut *tx).await.map_err(db_error)?;
        let mut heads = std::collections::HashMap::<String, Head>::new();
        for r in previous {
            let identity: String = r.get("identity_key");
            let mut head = Head {
                hash: r.get("structural_hash"),
                projection: r.get("structural_projection"),
                id: r.get("ingestion_id"),
            };
            if r.get::<String, _>("algorithm_version") != "http-structure-2" {
                // Upgrade only the current head under its existing lock; never scan history.
                head.projection = r
                    .get::<Option<Value>, _>("legacy_raw")
                    .as_ref()
                    .and_then(nexofolio_intake::http_projection);
                head.hash = head.projection.as_ref().map(|p| {
                    Sha256::digest(serde_json::to_vec(p).expect("JSON serializes")).to_vec()
                });
                sqlx::query("UPDATE ingestion_heads SET algorithm_version='http-structure-2',structural_projection=$4,structural_hash=$5 WHERE project_id=$1 AND environment_id=$2 AND identity_key=$3")
                    .bind(project).bind(environment_id).bind(&identity).bind(&head.projection).bind(&head.hash).execute(&mut *tx).await.map_err(db_error)?;
            }
            heads.insert(identity, head);
        }
        let capture_ids:Vec<Uuid>=sqlx::query_scalar("SELECT record_id FROM capture_receipts WHERE actor_id=$1 AND producer_id=$2 AND record_id=ANY($3)").bind(actor).bind(batch.source.instance_id).bind(&ids).fetch_all(&mut *tx).await.map_err(db_error)?;
        let mut to_insert = Vec::new();
        let mut receipts = Vec::new();
        for mut row in batch.records {
            // Resolve names/aliases to stable identity before validating immutable retries.
            row.content_hash =
                Sha256::digest([row.content_hash.as_slice(), environment_id.as_bytes()].concat())
                    .to_vec();
            if capture_ids.contains(&row.record_id) && !known.contains_key(&row.record_id) {
                receipts.push(RecordReceipt::reject(
                    row.index,
                    Some(row.record_id),
                    "IDEMPOTENCY_CONFLICT",
                ));
                continue;
            }
            let existing = known.get(&row.record_id);
            if let Some(existing) = existing {
                if existing.0 != row.content_hash {
                    receipts.push(RecordReceipt::reject(
                        row.index,
                        Some(row.record_id),
                        "IDEMPOTENCY_CONFLICT",
                    ));
                    continue;
                }
                receipts.push(RecordReceipt {
                    record_index: row.index,
                    record_id: Some(row.record_id),
                    status: if existing.1 == "accepted" {
                        ReceiptStatus::Accepted
                    } else {
                        ReceiptStatus::Ignored
                    },
                    reason_code: existing.2.clone(),
                    retryable: false,
                    ingestion_id: Some(existing.3),
                    replayed: true,
                });
                continue;
            }
            let duplicate = heads.get(&row.identity_key).filter(|head| {
                row.structural_hash.is_some()
                    && (head.hash == row.structural_hash
                        || head
                            .projection
                            .as_ref()
                            .zip(row.structural_projection.as_ref())
                            .is_some_and(|(known, incoming)| {
                                nexofolio_intake::http_projection_covers(known, incoming)
                            }))
            });
            let (id, status, code) = if let Some(head) = duplicate {
                (head.id, "ignored", "DUPLICATE_CURRENT_STRUCTURE")
            } else {
                let id = Uuid::new_v4();
                sqlx::query(r#"INSERT INTO ingestion_inbox(id,project_id,actor_id,producer_id,source_type,record_id,batch_id,environment_id,legacy_service_key,identity_key,structural_hash,raw_record,path_identity)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#)
      .bind(id).bind(project).bind(actor).bind(batch.source.instance_id).bind(&batch.source.r#type).bind(row.record_id).bind(batch.batch_id).bind(environment_id).bind(&batch.legacy_service_key).bind(&row.identity_key).bind(&row.structural_hash).bind(&row.raw).bind(serde_json::to_value(&row.path_identity).expect("serializes")).execute(&mut *tx).await.map_err(db_error)?;
                sqlx::query(r#"INSERT INTO ingestion_heads(project_id,environment_id,identity_key,algorithm_version,structural_hash,ingestion_id,structural_projection)
            VALUES($1,$2,$3,'http-structure-2',$4,$5,$6)
            ON CONFLICT(project_id,environment_id,identity_key) DO UPDATE
            SET structural_hash=excluded.structural_hash,ingestion_id=excluded.ingestion_id,algorithm_version=excluded.algorithm_version,structural_projection=excluded.structural_projection"#)
                  .bind(project).bind(environment_id).bind(&row.identity_key).bind(&row.structural_hash).bind(id).bind(&row.structural_projection).execute(&mut *tx).await.map_err(db_error)?;
                // Ignored weaker observations must not downgrade the head, including within a batch.
                heads.insert(
                    row.identity_key.clone(),
                    Head {
                        hash: row.structural_hash,
                        projection: row.structural_projection,
                        id,
                    },
                );
                (id, "accepted", "FORWARDED")
            };
            known.insert(
                row.record_id,
                (row.content_hash.clone(), status.into(), code.into(), id),
            );
            to_insert.push((row.record_id, row.content_hash, status, code, id));
            receipts.push(RecordReceipt {
                record_index: row.index,
                record_id: Some(row.record_id),
                status: if status == "accepted" {
                    ReceiptStatus::Accepted
                } else {
                    ReceiptStatus::Ignored
                },
                reason_code: code.into(),
                retryable: false,
                ingestion_id: Some(id),
                replayed: false,
            });
        }
        if !to_insert.is_empty() {
            let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
                "INSERT INTO ingestion_receipts(actor_id,producer_id,record_id,content_hash,status,reason_code,ingestion_id) ",
            );
            query.push_values(to_insert, |mut b, (record_id, hash, status, code, id)| {
                b.push_bind(actor)
                    .push_bind(batch.source.instance_id)
                    .push_bind(record_id)
                    .push_bind(hash)
                    .push_bind(status)
                    .push_bind(code)
                    .push_bind(id);
            });
            query.build().execute(&mut *tx).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(AdmissionResult {
            environment,
            receipts,
        })
    }
}
