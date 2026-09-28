use crate::wiring::Config;
use nexofolio_access_adapter::Postgres;
use nexofolio_common::Result;
use nexofolio_intake_adapter::PostgresLedger;

/// One lazy pool per module schema, all on the same database.
pub struct Databases {
    pub access: Postgres,
    pub intake: PostgresLedger,
}

impl Databases {
    pub fn new(config: &Config) -> Result<Self> {
        let (url, max, timeout) = (
            &config.database_url,
            config.database_max_connections,
            config.database_timeout,
        );
        Ok(Self {
            access: Postgres::new(url, max, timeout)?,
            intake: PostgresLedger::new(url, max, timeout)?,
        })
    }

    /// Explicit administration only; API and worker never migrate automatically.
    pub async fn migrate(&self) -> Result<()> {
        self.access.migrate().await?;
        self.intake.migrate().await
    }

    pub async fn close(&self) {
        self.access.close().await;
        self.intake.close().await;
    }
}
