use async_trait::async_trait;
use nexofolio_common::DatabaseProbe;
use nexofolio_common::{Error, Result, Secret};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct Postgres {
    pub(crate) pool: PgPool,
}

impl Postgres {
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
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Explicit administration only; API and worker never migrate automatically.
    pub async fn migrate(&self) -> Result<()> {
        let error = |_| Error::Unavailable {
            component: "database_migrations",
        };
        let ledger: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
                .fetch_one(&self.pool)
                .await
                .map_err(error)?;
        let legacy = if ledger.is_some() {
            sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version NOT IN (202609140001,202609140002))")
                .fetch_one(&self.pool).await.map_err(error)?
        } else {
            false
        };
        if legacy {
            // Frozen, checksummed migration ledger for databases made before the reset.
            // Never rewrite applied SQL or silently ignore missing migration versions.
            sqlx::migrate!("../../../migrations/legacy")
                .run(&self.pool)
                .await
        } else {
            sqlx::migrate!("./migrations").run(&self.pool).await
        }
        .map_err(|_| Error::Unavailable {
            component: "database_migrations",
        })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl DatabaseProbe for Postgres {
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
