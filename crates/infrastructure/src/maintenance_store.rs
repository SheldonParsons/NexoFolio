mod snapshot;
mod snapshot_inputs;
use crate::Postgres;
use async_trait::async_trait;
use nexofolio_application::{MaintenanceLease, MaintenanceStore};
use nexofolio_contracts::*;
use nexofolio_rebuild::snapshot_fields;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresMaintenance {
    pub(crate) database: Postgres,
    max_snapshot_bytes: usize,
}
impl PostgresMaintenance {
    pub fn with_snapshot_limit(database: Postgres, max_snapshot_bytes: usize) -> Self {
        Self {
            database,
            max_snapshot_bytes,
        }
    }
    pub fn new(database: Postgres) -> Self {
        Self {
            database,
            max_snapshot_bytes: 64 * 1024 * 1024,
        }
    }
}
pub(crate) fn id(v: impl ToString) -> Uuid {
    v.to_string().parse().expect("typed UUID")
}
pub(crate) fn db(_: sqlx::Error) -> Error {
    Error::Unavailable {
        component: "knowledge_maintenance",
    }
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| Error::Unavailable {
        component: "maintenance_contract",
    })
}
pub(crate) fn encode(v: &impl serde::Serialize) -> Value {
    serde_json::to_value(v).expect("serializes")
}
pub(crate) fn hash(v: &impl serde::Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(v).expect("serializes"))
    )
}
pub(crate) fn invalid(message: &str) -> Error {
    Error::InvalidInput {
        message: message.into(),
    }
}
pub(crate) fn page_check(page: u32, limit: u32) -> Result<()> {
    if page == 0 || page > 100000 || limit == 0 || limit > 100 {
        Err(invalid("invalid pagination"))
    } else {
        Ok(())
    }
}
pub(crate) use crate::project_access::authorize_project as auth;

const RUN: &str = "SELECT to_jsonb(m)-'snapshot'-'snapshot_sha256'-'settings_hash'-'generation'-'lease_until'-'actor_id'-'request_id' || jsonb_build_object('currently_published',EXISTS(SELECT 1 FROM project_catalogs p JOIN knowledge_releases k ON k.id=p.current_knowledge_version_id WHERE p.project_id=m.project_id AND k.source_run_id=m.id)) AS value FROM maintenance_runs m WHERE m.id=$1 AND m.project_id=$2";
impl PostgresMaintenance {
    pub(crate) async fn read_tx(
        &self,
        user: UserId,
        project: ProjectId,
    ) -> Result<Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        auth(&mut tx, user, project).await?;
        Ok(tx)
    }
    pub(crate) async fn run_tx(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        project: ProjectId,
        run: Uuid,
    ) -> Result<MaintenanceRun> {
        decode(
            sqlx::query(RUN)
                .bind(run)
                .bind(id(project))
                .fetch_optional(&mut **tx)
                .await
                .map_err(db)?
                .ok_or(Error::NotFound)?
                .get("value"),
        )
    }
    pub(crate) async fn fence(
        tx: &mut Transaction<'_, sqlx::Postgres>,
        lease: &MaintenanceLease,
    ) -> Result<()> {
        let found=sqlx::query("UPDATE maintenance_runs SET lease_until=clock_timestamp()+interval '5 minutes' WHERE id=$1 AND generation=$2 AND status='running' AND lease_until>clock_timestamp() RETURNING id")
  .bind(lease.run.id).bind(lease.generation).fetch_optional(&mut **tx).await.map_err(db)?;
        if found.is_none() {
            Err(Error::Conflict)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl MaintenanceStore for PostgresMaintenance {
    async fn start(
        &self,
        user: UserId,
        project: ProjectId,
        request: &StartMaintenance,
    ) -> Result<MaintenanceRun> {
        self.start_snapshot(user, project, request).await
    }
    async fn get(&self, user: UserId, project: ProjectId, run: Uuid) -> Result<MaintenanceRun> {
        let mut tx = self.read_tx(user, project).await?;
        let out = Self::run_tx(&mut tx, project, run).await?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
    async fn list(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<MaintenancePage> {
        page_check(page, limit)?;
        let mut tx = self.read_tx(user, project).await?;
        let total = sqlx::query_scalar("SELECT count(*) FROM maintenance_runs WHERE project_id=$1")
            .bind(id(project))
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let rows=sqlx::query(r#"SELECT to_jsonb(m)-'candidate'-'snapshot'-'snapshot_sha256'-'settings_hash'-'generation'-'lease_until'-'actor_id'-'request_id' || jsonb_build_object('candidate_available',candidate IS NOT NULL,'currently_published',EXISTS(SELECT 1
            FROM project_catalogs p
            JOIN knowledge_releases k ON k.id=p.current_knowledge_version_id
            WHERE p.project_id=m.project_id
            AND k.source_run_id=m.id)) AS value
            FROM maintenance_runs m
            WHERE project_id=$1
            ORDER BY created_at DESC,id
            LIMIT $2 OFFSET $3"#).bind(id(project)).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?;
        let items = rows
            .into_iter()
            .map(|r| decode(r.get("value")))
            .collect::<Result<_>>()?;
        tx.commit().await.map_err(db)?;
        Ok(MaintenancePage {
            items,
            total,
            page,
            limit,
        })
    }
    async fn snapshot(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
    ) -> Result<KnowledgeSnapshot> {
        let mut tx = self.read_tx(user, project).await?;
        let out = decode(
            sqlx::query_scalar(
                "SELECT snapshot FROM maintenance_runs WHERE id=$1 AND project_id=$2",
            )
            .bind(run)
            .bind(id(project))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(Error::NotFound)?,
        )?;
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
    async fn checkpoints(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        page: u32,
        limit: u32,
    ) -> Result<MaintenanceCheckpointPage> {
        page_check(page, limit)?;
        let mut tx = self.read_tx(user, project).await?;
        Self::run_tx(&mut tx, project, run).await?;
        let total =
            sqlx::query_scalar("SELECT count(*) FROM maintenance_checkpoints WHERE run_id=$1")
                .bind(run)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let items=sqlx::query_scalar("SELECT value FROM maintenance_checkpoints WHERE run_id=$1 ORDER BY created_at,id LIMIT $2 OFFSET $3").bind(run).bind(i64::from(limit)).bind(i64::from(page-1)*i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?.into_iter().map(decode).collect::<Result<_>>()?;
        tx.commit().await.map_err(db)?;
        Ok(MaintenanceCheckpointPage {
            items,
            total,
            page,
            limit,
        })
    }
    async fn claim(&self, settings_hash: &str) -> Result<Option<MaintenanceLease>> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        let row = sqlx::query(
            r#"SELECT id,project_id,settings_hash,snapshot
            FROM maintenance_runs
            WHERE status='pending'
            OR (status='running'
            AND lease_until<=clock_timestamp())
            ORDER BY created_at FOR UPDATE SKIP LOCKED
            LIMIT 1"#,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some(row) = row else {
            tx.commit().await.map_err(db)?;
            return Ok(None);
        };
        let run_id: Uuid = row.get("id");
        let old: Option<String> = row.get("settings_hash");
        if old.as_ref().is_some_and(|v| v != settings_hash) {
            sqlx::query("UPDATE maintenance_runs SET status='incomplete',error_code='MODEL_CONFIGURATION_CHANGED',lease_until=NULL WHERE id=$1").bind(run_id).execute(&mut *tx).await.map_err(db)?;
            tx.commit().await.map_err(db)?;
            return Ok(None);
        }
        let generation:i64=sqlx::query_scalar("UPDATE maintenance_runs SET status='running',generation=generation+1,lease_until=clock_timestamp()+interval '5 minutes',settings_hash=$2 WHERE id=$1 RETURNING generation").bind(run_id).bind(settings_hash).fetch_one(&mut *tx).await.map_err(db)?;
        let run = Self::run_tx(
            &mut tx,
            row.get::<Uuid, _>("project_id")
                .to_string()
                .parse()
                .unwrap(),
            run_id,
        )
        .await?;
        let snapshot = decode(row.get("snapshot"))?;
        tx.commit().await.map_err(db)?;
        Ok(Some(MaintenanceLease {
            run,
            snapshot,
            generation,
            settings_hash: settings_hash.into(),
        }))
    }
    async fn call_count(&self, lease: &MaintenanceLease) -> Result<u32> {
        let n:i32=sqlx::query_scalar("SELECT model_calls FROM maintenance_runs WHERE id=$1 AND generation=$2 AND status='running'").bind(lease.run.id).bind(lease.generation).fetch_optional(&self.database.pool).await.map_err(db)?.ok_or(Error::Conflict)?;
        Ok(n as u32)
    }
    async fn renew(&self, lease: &MaintenanceLease) -> Result<()> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        Self::fence(&mut tx, lease).await?;
        tx.commit().await.map_err(db)
    }
    async fn checkpoint(
        &self,
        lease: &MaintenanceLease,
        c: &MaintenanceCheckpoint,
        coverage: &ReviewCoverage,
    ) -> Result<()> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        Self::fence(&mut tx, lease).await?;
        sqlx::query("INSERT INTO maintenance_checkpoints(run_id,id,value) VALUES($1,$2,$3) ON CONFLICT(run_id,id) DO UPDATE SET value=excluded.value").bind(lease.run.id).bind(&c.id).bind(encode(c)).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE maintenance_runs SET phase=$2,coverage=$3,read_count=(SELECT count(*) FROM maintenance_checkpoints WHERE run_id=$1 AND value->>'phase'='readback') WHERE id=$1").bind(lease.run.id).bind(&c.phase).bind(encode(coverage)).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    async fn saved_checkpoints(
        &self,
        lease: &MaintenanceLease,
    ) -> Result<Vec<MaintenanceCheckpoint>> {
        sqlx::query_scalar("SELECT value FROM maintenance_checkpoints WHERE run_id=$1 ORDER BY id")
            .bind(lease.run.id)
            .fetch_all(&self.database.pool)
            .await
            .map_err(db)?
            .into_iter()
            .map(decode)
            .collect()
    }
    async fn begin_call(
        &self,
        lease: &MaintenanceLease,
        request: &Value,
        limit: u32,
    ) -> Result<Uuid> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        Self::fence(&mut tx, lease).await?;
        let changed=sqlx::query("UPDATE maintenance_runs SET model_calls=model_calls+1 WHERE id=$1 AND model_calls<$2 RETURNING id").bind(lease.run.id).bind(limit as i32).fetch_optional(&mut *tx).await.map_err(db)?;
        if changed.is_none() {
            return Err(invalid("MODEL_CALL_BUDGET_EXHAUSTED"));
        }
        let call = Uuid::new_v4();
        sqlx::query("INSERT INTO maintenance_calls(id,run_id,generation,request,request_sha256) VALUES($1,$2,$3,$4,$5)").bind(call).bind(lease.run.id).bind(lease.generation).bind(request).bind(hash(request)).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(call)
    }
    async fn end_call(&self, lease: &MaintenanceLease, call: Uuid, response: &Value) -> Result<()> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        Self::fence(&mut tx, lease).await?;
        sqlx::query("UPDATE maintenance_calls SET response=$4,finished_at=clock_timestamp() WHERE id=$1 AND run_id=$2 AND generation=$3").bind(call).bind(lease.run.id).bind(lease.generation).bind(response).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    async fn finish(
        &self,
        lease: &MaintenanceLease,
        candidate: Option<&MaintenanceCandidate>,
        code: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.database.pool.begin().await.map_err(db)?;
        Self::fence(&mut tx, lease).await?;
        let ready = candidate.is_some_and(|c| {
            c.coverage.complete && c.issues.is_empty() && c.review.structurally_valid
        }) && code.is_none();
        sqlx::query("UPDATE maintenance_runs SET status=$2,phase=$3,candidate=$4,error_code=$5,lease_until=NULL WHERE id=$1").bind(lease.run.id).bind(if ready{"ready"}else{"incomplete"}).bind(if ready{"ready"}else{"incomplete"}).bind(candidate.map(encode)).bind(code).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
}
pub(crate) fn mark_stale(annotations: &mut [SemanticAnnotation], interfaces: &[CatalogInterface]) {
    for a in annotations {
        a.stale = a.basis.iter().any(|b| {
            !interfaces.iter().any(|i| {
                i.interface_id == b.interface_id
                    && i.environments.iter().any(|e| {
                        e.environment_id == b.environment_id && e.revision_id == b.revision_id
                    })
            })
        });
    }
}

#[async_trait]
impl nexofolio_application::MaintenanceSources for crate::PostgresCaptureStore {
    async fn observation(&self, source: &SnapshotSource) -> Result<Value> {
        let hash = source.raw_hash.as_ref().ok_or(Error::Unavailable {
            component: "pinned_evidence_original",
        })?;
        let bytes = self.blobs.get(source.project_id, hash).await?;
        if format!("{:x}", Sha256::digest(&bytes)) != *hash {
            return Err(invalid("FROZEN_SOURCE_HASH_MISMATCH"));
        }
        let original: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid original evidence"))?;
        Ok(
            serde_json::json!({"event_id":source.event_id,"project_id":source.project_id,"environment_id":source.environment_id,"actor_id":source.actor_id,"producer_id":source.producer_id,"record_id":source.record_id,"captured_at":source.captured_at,"raw_hash":hash,"context":source.context,"kind":source.kind,"evidence_status":source.evidence_status,"evidence_coverage":source.evidence_coverage,"original":original}),
        )
    }
    async fn image(&self, project: ProjectId, asset: Uuid) -> Result<Value> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let row=sqlx::query("SELECT b.sha256,b.media_type FROM capture_assets a JOIN capture_blobs b ON b.project_id=a.project_id AND b.sha256=a.sha256 WHERE a.project_id=$1 AND a.id=$2").bind(id(project)).bind(asset).fetch_optional(&self.database.pool).await.map_err(db)?.ok_or(Error::NotFound)?;
        let bytes = self
            .blobs
            .get(project, &row.get::<String, _>("sha256"))
            .await?;
        Ok(Value::String(format!(
            "data:{};base64,{}",
            row.get::<String, _>("media_type"),
            STANDARD.encode(bytes)
        )))
    }
}
