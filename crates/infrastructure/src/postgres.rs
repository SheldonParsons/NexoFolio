use async_trait::async_trait;
use nexofolio_application::DatabaseProbe;
use nexofolio_contracts::{Error, Result, Secret};
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
            return Err(Error::InvalidInput {
                message: "database pool limits must be positive".into(),
            });
        }
        if !url.expose().starts_with("postgres://") && !url.expose().starts_with("postgresql://") {
            return Err(Error::InvalidInput {
                message: "DATABASE_URL must be a PostgreSQL URL".into(),
            });
        }
        let options = PgConnectOptions::from_str(url.expose())
            .map_err(|_| Error::InvalidInput {
                message: "DATABASE_URL is invalid".into(),
            })?
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Explicit administration only; API and worker never migrate automatically.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations")
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
