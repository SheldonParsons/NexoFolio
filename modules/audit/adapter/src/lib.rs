//! Audit owns its schema and a separate one-connection pool.
mod access;
use async_trait::async_trait;
use nexofolio_audit_contracts::{AuditStore, Event, Filter, Limits};
use nexofolio_common::{Error, Result, Secret};
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct PostgresAudit {
    pool: PgPool,
}
fn unavailable(_: sqlx::Error) -> Error {
    Error::Unavailable { component: "audit" }
}
impl PostgresAudit {
    pub fn new(url: &Secret) -> Result<Self> {
        let options = PgConnectOptions::from_str(url.expose())
            .map_err(|_| Error::invalid("DATABASE_URL is invalid"))?
            .options([
                ("search_path", "audit"),
                ("statement_timeout", "2000"),
                ("lock_timeout", "500"),
            ])
            .disable_statement_logging();
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(2))
                .connect_lazy_with(options),
        })
    }
    pub async fn migrate(&self) -> Result<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.create_schema("audit");
        migrator.dangerous_set_table_name("audit._sqlx_migrations".to_string());
        migrator
            .run(&self.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "audit_migrations",
            })
    }
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl AuditStore for PostgresAudit {
    async fn write(&self, events: &[Event], limits: &Limits) -> Result<()> {
        if events.len() > 256 || events.iter().any(|e| !e.bounded()) {
            return Err(Error::invalid("invalid audit batch"));
        }
        let mut details = Vec::new();
        let mut access = Vec::new();
        for event in events {
            if matches!(event.kind.as_str(), "page" | "api") {
                let hour = event.at.timestamp().div_euclid(3600) * 3600;
                let key = format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&(
                            hour,
                            &event.kind,
                            &event.actor,
                            &event.project,
                            &event.route,
                            &event.method,
                            event.status
                        ))
                        .expect("audit key")
                    )
                );
                access.push(serde_json::json!({"key":key,"hour":hour,"kind":event.kind,
                    "actor":event.actor,"project":event.project,"route":event.route,"method":event.method,
                    "status":event.status,"duration_ms":event.duration_ms}));
            } else {
                details.push(event);
            }
        }
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        // Serializes counters/eviction across processes; never locks business tables.
        sqlx::query("SELECT id FROM counters WHERE id=true FOR UPDATE")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("WITH added AS (INSERT INTO events(at,kind,actor,request_id,project,data) SELECT (v->>'at')::timestamptz,v->>'kind',v->>'actor',v->>'request_id',v->>'project',v FROM jsonb_array_elements($1::jsonb) v RETURNING id) UPDATE counters SET events=events+(SELECT count(*) FROM added) WHERE id=true")
            .bind(serde_json::to_string(&details).expect("audit details")).execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("WITH added AS (INSERT INTO buckets(key,hour,kind,actor,project,route,method,status,hits,duration_ms,max_duration_ms) SELECT key,to_timestamp(hour),kind,actor,project,route,method,status,count(*),sum(duration_ms),max(duration_ms) FROM jsonb_to_recordset($1::jsonb) AS v(key text,hour bigint,kind text,actor text,project text,route text,method text,status integer,duration_ms bigint) GROUP BY key,hour,kind,actor,project,route,method,status ON CONFLICT(key) DO UPDATE SET hits=buckets.hits+EXCLUDED.hits,duration_ms=buckets.duration_ms+EXCLUDED.duration_ms,max_duration_ms=greatest(buckets.max_duration_ms,EXCLUDED.max_duration_ms) RETURNING (xmax=0) AS fresh) UPDATE counters SET buckets=buckets+(SELECT count(*) FROM added WHERE fresh) WHERE id=true")
            .bind(serde_json::to_string(&access).expect("audit access")).execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("WITH removed AS (DELETE FROM events WHERE id IN (SELECT id FROM events ORDER BY at,id LIMIT (SELECT greatest(events-$1,0) FROM counters)) RETURNING id) UPDATE counters SET events=events-(SELECT count(*) FROM removed), evicted_events=evicted_events+(SELECT count(*) FROM removed) WHERE id=true")
            .bind(i64::from(limits.events)).execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("WITH removed AS (DELETE FROM buckets WHERE key IN (SELECT key FROM buckets ORDER BY hour,key LIMIT (SELECT greatest(buckets-$1,0) FROM counters)) RETURNING key) UPDATE counters SET buckets=buckets-(SELECT count(*) FROM removed), evicted_buckets=evicted_buckets+(SELECT count(*) FROM removed) WHERE id=true")
            .bind(i64::from(limits.buckets)).execute(&mut *tx).await.map_err(unavailable)?;
        access::write(&mut tx, events, limits).await?;
        tx.commit().await.map_err(unavailable)
    }
    async fn purge(&self, limits: &Limits) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SELECT id FROM counters WHERE id=true FOR UPDATE")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        // Small deletions avoid long transactions. Repeated each minute while API is running.
        sqlx::query("WITH removed AS (DELETE FROM events WHERE id IN (SELECT id FROM events WHERE at < now() - (CASE WHEN kind='error' THEN $1 ELSE $2 END * interval '1 day') ORDER BY at,id LIMIT 5000) RETURNING id) UPDATE counters SET events=events-(SELECT count(*) FROM removed) WHERE id=true")
            .bind(i32::try_from(limits.error_days).unwrap_or(30)).bind(i32::try_from(limits.operation_days).unwrap_or(90))
            .execute(&mut *tx).await.map_err(unavailable)?;
        sqlx::query("WITH removed AS (DELETE FROM buckets WHERE key IN (SELECT key FROM buckets WHERE hour < now() - ($1 * interval '1 day') ORDER BY hour,key LIMIT 5000) RETURNING key) UPDATE counters SET buckets=buckets-(SELECT count(*) FROM removed) WHERE id=true")
            .bind(i32::try_from(limits.access_days).unwrap_or(30)).execute(&mut *tx).await.map_err(unavailable)?;
        access::purge(&mut tx, limits).await?;
        tx.commit().await.map_err(unavailable)
    }
    async fn access_report(
        &self,
        query: &nexofolio_audit_contracts::AccessQuery,
    ) -> Result<serde_json::Value> {
        self.read_access(query).await
    }
    async fn curation_events(
        &self,
        project: &str,
        candidate: Option<&str>,
        task: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.read_candidate_events(project, candidate, task).await
    }
    async fn report(&self, f: &Filter) -> Result<serde_json::Value> {
        let limit = i64::from(f.limit.clamp(1, 200));
        let events: Vec<String> = sqlx::query_scalar("SELECT data::text FROM events WHERE ($1::text IS NULL OR actor=$1) AND ($2::text IS NULL OR request_id=$2) AND ($3::text IS NULL OR project=$3) AND ($4::text IS NULL OR kind=$4) AND ($6::text IS NULL OR data->>'route'=$6) AND ($7::timestamptz IS NULL OR at >= $7) AND ($8::timestamptz IS NULL OR at <= $8) ORDER BY at DESC,id DESC LIMIT $5")
            .bind(&f.actor).bind(&f.request_id).bind(&f.project).bind(&f.kind).bind(limit).bind(&f.route).bind(f.since).bind(f.until).fetch_all(&self.pool).await.map_err(unavailable)?;
        let buckets: Vec<String> = sqlx::query_scalar("SELECT to_jsonb(b)::text FROM buckets b WHERE ($1::text IS NULL OR actor=$1) AND ($2::text IS NULL OR project=$2) AND ($3::text IS NULL OR kind=$3) AND ($5::text IS NULL OR route=$5) AND ($6::timestamptz IS NULL OR hour >= date_trunc('hour',$6)) AND ($7::timestamptz IS NULL OR hour <= $7) ORDER BY hour DESC,hits DESC,key LIMIT $4")
            .bind(&f.actor).bind(&f.project).bind(&f.kind).bind(limit).bind(&f.route).bind(f.since).bind(f.until).fetch_all(&self.pool).await.map_err(unavailable)?;
        let summary: Vec<String> = sqlx::query_scalar("SELECT jsonb_build_object('route',route,'hits',sum(hits),'actors',count(DISTINCT actor))::text FROM buckets WHERE ($1::text IS NULL OR actor=$1) AND ($2::text IS NULL OR project=$2) AND ($3::text IS NULL OR kind=$3) AND ($4::text IS NULL OR route=$4) AND ($5::timestamptz IS NULL OR hour >= date_trunc('hour',$5)) AND ($6::timestamptz IS NULL OR hour <= $6) GROUP BY route ORDER BY sum(hits) DESC,route LIMIT 50")
            .bind(&f.actor).bind(&f.project).bind(&f.kind).bind(&f.route).bind(f.since).bind(f.until).fetch_all(&self.pool).await.map_err(unavailable)?;
        let stats: String = sqlx::query_scalar("SELECT (to_jsonb(c) || jsonb_build_object('bytes',pg_total_relation_size('events')+pg_total_relation_size('buckets')+pg_total_relation_size('access_events')))::text FROM counters c WHERE id=true")
            .fetch_one(&self.pool).await.map_err(unavailable)?;
        let parse = |s: String| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default();
        Ok(
            serde_json::json!({"events":events.into_iter().map(parse).collect::<Vec<_>>(),"buckets":if f.request_id.is_some() { Vec::new() } else { buckets.into_iter().map(parse).collect::<Vec<_>>() },"summary":if f.request_id.is_some() { Vec::new() } else { summary.into_iter().map(parse).collect::<Vec<_>>() },"storage":parse(stats)}),
        )
    }
}

impl nexofolio_common::PoolTelemetry for PostgresAudit {
    fn pool_snapshots(&self) -> Vec<nexofolio_common::PoolSnapshot> {
        vec![nexofolio_common::PoolSnapshot {
            name: "audit",
            size: self.pool.size(),
            idle: self.pool.num_idle(),
            configured_max: self.pool.options().get_max_connections(),
            waiting: None,
            acquire_timeouts: None,
        }]
    }
}
