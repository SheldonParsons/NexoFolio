//! Pool administration and entry to a knowledge-owned transaction.
use async_trait::async_trait;
use nexofolio_common::{DatabaseProbe, Error, Result, Secret};
use nexofolio_knowledge_contracts::{KnowledgeStore, KnowledgeTx, StoreResult};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

mod apply_buffer;
mod candidate_diagnostics;
mod candidates;
mod codec;
mod folders;
mod frozen;
mod lifecycle;
mod links;
mod notes;
mod operations;
mod placements;
mod retention;
mod retention_references;
mod review;
mod revisions;
mod storage;
pub use retention::RetentionReport;
mod rounds;
mod search;
mod tasks;
mod transaction;
mod value_archives;
use codec::unavailable;
use transaction::PostgresTx;

/// Everything knowledge stores lives in this schema. Connections put it alone
/// on the `search_path`, so SQL in this crate stays unqualified.
const SCHEMA: &str = "knowledge";
pub(crate) const PROJECT_LOCK: i32 = 0x6b6e_0001;

#[derive(Clone)]
pub struct PostgresKnowledge {
    pool: PgPool,
}

impl PostgresKnowledge {
    /// Lazy connections keep liveness available during database outages.
    pub fn new(url: &Secret, max_connections: u32, timeout: Duration) -> Result<Self> {
        if max_connections == 0 || timeout.is_zero() {
            return Err(Error::invalid("database pool limits must be positive"));
        }
        if !url.expose().starts_with("postgres://") && !url.expose().starts_with("postgresql://") {
            return Err(Error::invalid("DATABASE_URL must be a PostgreSQL URL"));
        }
        let options = PgConnectOptions::from_str(url.expose())
            .map_err(|_| Error::invalid("DATABASE_URL is invalid"))?
            .options([("search_path", SCHEMA)])
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Explicit administration only; the api never migrates automatically.
    pub async fn migrate(&self) -> Result<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.create_schema(SCHEMA);
        migrator.dangerous_set_table_name(format!("{SCHEMA}._sqlx_migrations"));
        migrator
            .run(&self.pool)
            .await
            .map_err(|_| Error::Unavailable {
                component: "database_migrations",
            })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl DatabaseProbe for PostgresKnowledge {
    async fn check(&self) -> Result<()> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(|_| Error::Unavailable {
                component: "database",
            })
    }
}

#[async_trait]
impl KnowledgeStore for PostgresKnowledge {
    async fn begin_read(&self) -> StoreResult<Box<dyn KnowledgeTx>> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        Ok(Box::new(PostgresTx {
            tx,
            apply_buffer: None,
        }))
    }
    async fn begin(&self) -> StoreResult<Box<dyn KnowledgeTx>> {
        let tx = self.pool.begin().await.map_err(unavailable)?;
        Ok(Box::new(PostgresTx {
            tx,
            apply_buffer: None,
        }))
    }
}

impl nexofolio_common::PoolTelemetry for PostgresKnowledge {
    fn pool_snapshots(&self) -> Vec<nexofolio_common::PoolSnapshot> {
        vec![nexofolio_common::PoolSnapshot {
            name: "knowledge",
            size: self.pool.size(),
            idle: self.pool.num_idle(),
            configured_max: self.pool.options().get_max_connections(),
            waiting: None,
            acquire_timeouts: None,
        }]
    }
}

impl PostgresTx {
    pub(super) async fn db_lock_project(
        &mut self,
        project: nexofolio_common::ProjectId,
    ) -> nexofolio_knowledge_contracts::StoreResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(project.to_string())
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }
}
