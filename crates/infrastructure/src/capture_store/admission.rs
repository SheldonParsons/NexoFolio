use super::*;
#[async_trait]
impl CaptureAdmissionStore for PostgresCaptureStore {
    async fn admit_captures(
        &self,
        actor: UserId,
        instance: &str,
        batch: CaptureBatch,
        mut records: Vec<PreparedCapture>,
    ) -> Result<CaptureBatchReceipt> {
        let mut tx = self.database.pool.begin().await.map_err(err)?;
        sqlx::query("SET LOCAL lock_timeout='3s'")
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        authorize(&mut tx, actor, batch.project_id).await?;
        let row =
            sqlx::query("SELECT path_policy FROM projects WHERE id=$1 AND instance=$2 FOR SHARE")
                .bind(uuid(batch.project_id))
                .bind(instance)
                .fetch_optional(&mut *tx)
                .await
                .map_err(err)?
                .ok_or(Error::Forbidden)?;
        let policy: PathPolicy =
            serde_json::from_value(row.get("path_policy")).map_err(|_| Error::Unavailable {
                component: "path_policy",
            })?;
        if !policy.valid() {
            return Err(Error::Unavailable {
                component: "path_policy",
            });
        }
        let environment =
            crate::environments::resolve(&mut tx, uuid(batch.project_id), &batch.environment)
                .await?;
        let env = uuid(environment.id);
        let mut locks = std::collections::BTreeSet::new();
        for r in &mut records {
            r.content_hash =
                Sha256::digest([r.content_hash.as_slice(), env.as_bytes()].concat()).to_vec();
            let content =
                Sha256::digest(serde_json::to_vec(&r.record.payload).expect("serializes"))
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
            locks.insert(format!("blob:{}:{content}", batch.project_id));
            locks.insert(format!(
                "record:{actor}:{}:{}",
                batch.source.instance_id, r.record.record_id
            ));
            if let Some(http) = &mut r.http {
                nexofolio_intake::apply_path_policy(http, &policy);
                locks.insert(format!(
                    "head:{}:{env}:{}",
                    batch.project_id, http.identity_key
                ));
            }
        }
        sqlx::query("SELECT pg_advisory_xact_lock(k) FROM (SELECT DISTINCT hashtextextended(x,0) AS k FROM unnest($1::text[]) x ORDER BY k) ordered").bind(locks.into_iter().collect::<Vec<_>>()).execute(&mut *tx).await.map_err(err)?;
        let ids: Vec<_> = records.iter().map(|r| r.record.record_id).collect();
        // Read the bounded batch's receipts and comparison heads once, under their ordered locks.
        let mut known = std::collections::HashMap::<Uuid, (Vec<u8>, CaptureReceipt)>::new();
        let legacy=sqlx::query("SELECT record_id,content_hash,status,reason_code,ingestion_id FROM ingestion_receipts WHERE actor_id=$1 AND producer_id=$2 AND record_id=ANY($3)").bind(uuid(actor)).bind(batch.source.instance_id).bind(&ids).fetch_all(&mut *tx).await.map_err(err)?;
        for p in legacy {
            let id = p.get("record_id");
            let ignored = p.get::<String, _>("status") == "ignored";
            known.insert(
                id,
                (
                    p.get("content_hash"),
                    CaptureReceipt {
                        record_index: 0,
                        record_id: Some(id),
                        status: if ignored {
                            CaptureReceiptStatus::Ignored
                        } else {
                            CaptureReceiptStatus::Accepted
                        },
                        reason_code: p.get("reason_code"),
                        retryable: false,
                        observation_id: None,
                        ingestion_id: Some(p.get("ingestion_id")),
                        structure: if ignored { "duplicate" } else { "accepted" }.into(),
                        replayed: false,
                    },
                ),
            );
        }
        let prior=sqlx::query("SELECT record_id,content_hash,result FROM capture_receipts WHERE actor_id=$1 AND producer_id=$2 AND record_id=ANY($3)").bind(uuid(actor)).bind(batch.source.instance_id).bind(&ids).fetch_all(&mut *tx).await.map_err(err)?;
        for p in prior {
            known.insert(
                p.get("record_id"),
                (
                    p.get("content_hash"),
                    serde_json::from_value(p.get("result")).map_err(|_| Error::Unavailable {
                        component: "capture_receipt",
                    })?,
                ),
            );
        }
        let new_count = ids
            .into_iter()
            .filter(|id| !known.contains_key(id))
            .collect::<std::collections::HashSet<_>>()
            .len() as i64;
        let keys: Vec<_> = records
            .iter()
            .filter_map(|r| r.http.as_ref().map(|h| h.identity_key.clone()))
            .collect();
        let mut heads =
            crate::ingestion_heads::load(&mut tx, uuid(batch.project_id), env, &keys).await?;
        let mut blobs = std::collections::HashMap::<String, nexofolio_evidence::BlobRef>::new();
        if new_count > 0 {
            let reserved = sqlx::query(
                r#"INSERT INTO capture_backlog(project_id,pending)
            VALUES($1,$2)
            ON CONFLICT(project_id) DO UPDATE
            SET pending=capture_backlog.pending+excluded.pending
            WHERE capture_backlog.pending+excluded.pending<=50000
            RETURNING project_id"#,
            )
            .bind(uuid(batch.project_id))
            .bind(new_count)
            .fetch_optional(&mut *tx)
            .await
            .map_err(err)?;
            if reserved.is_none() {
                return Err(Error::Unavailable {
                    component: "capture_backpressure",
                });
            }
        }
        let mut results = Vec::new();
        for r in records {
            let id = r.record.record_id;
            if let Some((hash, receipt)) = known.get(&id) {
                if *hash != r.content_hash {
                    results.push(reject(r.index, id, "IDEMPOTENCY_CONFLICT"));
                    continue;
                }
                let mut receipt = receipt.clone();
                receipt.record_index = r.index;
                receipt.replayed = true;
                results.push(receipt);
                continue;
            }
            let (ingestion, structure) = if let Some(http) = &r.http {
                let duplicate = heads
                    .get(&http.identity_key)
                    .filter(|head| head.covers(http));
                if let Some(head) = duplicate {
                    (Some(head.id), "duplicate")
                } else {
                    let ingestion = Uuid::new_v4();
                    sqlx::query(r#"INSERT INTO ingestion_inbox(id,project_id,actor_id,producer_id,source_type,record_id,batch_id,environment_id,legacy_service_key,identity_key,structural_hash,raw_record,path_identity)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#)
      .bind(ingestion).bind(uuid(batch.project_id)).bind(uuid(actor)).bind(batch.source.instance_id).bind(&batch.source.r#type).bind(id).bind(batch.batch_id).bind(env).bind(&batch.service_key).bind(&http.identity_key).bind(&http.structural_hash).bind(&r.raw).bind(serde_json::to_value(&http.path_identity).expect("serializes")).execute(&mut *tx).await.map_err(err)?;
                    let head = crate::ingestion_heads::replace(
                        &mut tx,
                        uuid(batch.project_id),
                        env,
                        ingestion,
                        http,
                    )
                    .await?;
                    heads.insert(http.identity_key.clone(), head);
                    (Some(ingestion), "accepted")
                }
            } else {
                (None, "not_applicable")
            };
            let bytes = serde_json::to_vec(&r.record.payload).expect("serializes");
            let content = format!("{:x}", Sha256::digest(&bytes));
            let blob = if let Some(blob) = blobs.get(&content) {
                blob.clone()
            } else {
                let blob = self
                    .blobs
                    .put(batch.project_id, bytes, "application/json")
                    .await?;
                sqlx::query("INSERT INTO capture_blobs(project_id,sha256,bytes,media_type) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(uuid(batch.project_id)).bind(&blob.sha256).bind(blob.bytes as i64).bind(&blob.media_type).execute(&mut *tx).await.map_err(err)?;
                blobs.insert(content, blob.clone());
                blob
            };
            let event = Uuid::new_v4();
            sqlx::query(r#"INSERT INTO capture_events(id,project_id,environment_id,actor_id,producer_id,source_type,record_id,kind,payload_version,captured_at,context,raw_hash,content_hash,ingestion_id,structure)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::timestamptz,$11,$12,$13,$14,$15)"#)
    .bind(event).bind(uuid(batch.project_id)).bind(env).bind(uuid(actor)).bind(batch.source.instance_id).bind(&batch.source.r#type).bind(id).bind(string_kind(&r.record.kind)).bind(&r.record.payload_version).bind(&r.record.captured_at).bind(r.record.context.as_ref().map(|c|serde_json::to_value(c).expect("serializes"))).bind(&blob.sha256).bind(&r.content_hash).bind(ingestion).bind(structure).execute(&mut *tx).await.map_err(err)?;
            let receipt = CaptureReceipt {
                record_index: r.index,
                record_id: Some(id),
                status: CaptureReceiptStatus::Accepted,
                reason_code: "OBSERVATION_STORED".into(),
                retryable: false,
                observation_id: Some(event),
                ingestion_id: ingestion,
                structure: structure.into(),
                replayed: false,
            };
            sqlx::query("INSERT INTO capture_receipts(actor_id,producer_id,record_id,content_hash,result) VALUES($1,$2,$3,$4,$5)").bind(uuid(actor)).bind(batch.source.instance_id).bind(id).bind(&r.content_hash).bind(serde_json::to_value(&receipt).expect("serializes")).execute(&mut *tx).await.map_err(err)?;
            if batch.schema_version != "3" {
                sqlx::query("INSERT INTO ingestion_receipts(actor_id,producer_id,record_id,content_hash,status,reason_code,ingestion_id) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
                    .bind(uuid(actor)).bind(batch.source.instance_id).bind(id).bind(&r.content_hash).bind(if structure=="duplicate"{"ignored"}else{"accepted"}).bind(if structure=="duplicate"{"DUPLICATE_CURRENT_STRUCTURE"}else{"FORWARDED"}).bind(ingestion).execute(&mut *tx).await.map_err(err)?;
            }
            known.insert(id, (r.content_hash.clone(), receipt.clone()));
            results.push(receipt);
        }
        tx.commit().await.map_err(err)?;
        Ok(CaptureBatchReceipt {
            schema_version: batch.schema_version,
            batch_id: batch.batch_id,
            environment: Some(environment),
            results,
        })
    }
}
fn reject(index: usize, id: Uuid, code: &str) -> CaptureReceipt {
    CaptureReceipt {
        record_index: index,
        record_id: Some(id),
        status: CaptureReceiptStatus::Rejected,
        reason_code: code.into(),
        retryable: false,
        observation_id: None,
        ingestion_id: None,
        structure: "not_applicable".into(),
        replayed: false,
    }
}
