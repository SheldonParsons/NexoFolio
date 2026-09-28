use nexofolio_backend::wiring::{
    Config, Databases, init_logging, install_shutdown_handler, run_worker,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let databases = Databases::new(&config)?;
    let shutdown = CancellationToken::new();
    let signals = install_shutdown_handler(shutdown.clone())?;
    run_worker(
        shutdown,
        Arc::new(databases.intake.clone()),
        Arc::new(databases.observe.clone()),
    )
    .await;
    signals.abort();
    databases.close().await;
    Ok(())
}
