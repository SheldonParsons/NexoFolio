use nexofolio_backend::wiring::{Config, init_logging, install_shutdown_handler, run_worker};
use nexofolio_infrastructure::Postgres;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let database = Postgres::new(
        &config.database_url,
        config.database_max_connections,
        config.database_timeout,
    )?;
    let shutdown = CancellationToken::new();
    let signals = install_shutdown_handler(shutdown.clone())?;
    run_worker(shutdown).await;
    signals.abort();
    database.close().await;
    Ok(())
}
