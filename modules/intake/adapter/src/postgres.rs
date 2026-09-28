use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{DatabaseProbe, Error, Result, Secret};
use nexofolio_intake_contracts::{BatchLedger, LedgerEntry, LedgerError, Receipt};
use sqlx::{
    ConnectOptions, PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

/// Everything intake stores lives in this schema. Connections put it alone on
/// the `search_path`, so SQL in this crate stays unqualified.
const SCHEMA: &str = "intake";

#[derive(Clone)]
pub struct PostgresLedger {
    pool: PgPool,
}

impl PostgresLedger {
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

    /// Explicit administration only; API and worker never migrate automatically.
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
impl DatabaseProbe for PostgresLedger {
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

fn unavailable<E>(_: E) -> LedgerError {
    LedgerError::Unavailable
}

#[async_trait]
impl BatchLedger for PostgresLedger {
    async fn find(&self, batch_id: Uuid) -> std::result::Result<Option<LedgerEntry>, LedgerError> {
        let row = sqlx::query(
            "SELECT content_hash,platform,receipt,recorded_at FROM batches WHERE batch_id=$1",
        )
        .bind(batch_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let hash: Vec<u8> = row.try_get("content_hash").map_err(unavailable)?;
        let receipt: serde_json::Value = row.try_get("receipt").map_err(unavailable)?;
        Ok(Some(LedgerEntry {
            batch_id,
            content_hash: hash.try_into().map_err(unavailable)?,
            platform: row.try_get("platform").map_err(unavailable)?,
            receipt: serde_json::from_value::<Receipt>(receipt).map_err(unavailable)?,
            recorded_at: row.try_get("recorded_at").map_err(unavailable)?,
        }))
    }

    async fn record(&self, entry: &LedgerEntry) -> std::result::Result<(), LedgerError> {
        let receipt = serde_json::to_value(&entry.receipt).map_err(unavailable)?;
        sqlx::query(
            "INSERT INTO batches(batch_id,content_hash,platform,receipt,recorded_at) \
             VALUES($1,$2,$3,$4,$5) ON CONFLICT(batch_id) DO NOTHING",
        )
        .bind(entry.batch_id)
        .bind(entry.content_hash.as_slice())
        .bind(&entry.platform)
        .bind(receipt)
        .bind(entry.recorded_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(unavailable)
    }

    async fn purge(&self, before: DateTime<Utc>) -> std::result::Result<u64, LedgerError> {
        sqlx::query("DELETE FROM batches WHERE recorded_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected())
            .map_err(unavailable)
    }
}
