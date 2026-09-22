use nexofolio_access_adapter::{Postgres, Unconfigured};
use nexofolio_backend::{
    http,
    wiring::{Config, init_logging, install_shutdown_handler},
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let database = Arc::new(Postgres::new(
        &config.database_url,
        config.database_max_connections,
        config.database_timeout,
    )?);
    let listener = TcpListener::bind(config.bind_addr).await?;
    let shutdown = CancellationToken::new();
    let signals = install_shutdown_handler(shutdown.clone())?;
    let access = nexofolio_backend::wiring::build_access(&config, (*database).clone())?;
    let router = http::router_with_access(
        &config,
        database.clone(),
        Arc::new(Unconfigured),
        shutdown.clone(),
        access,
    );
    tracing::info!(address = %listener.local_addr()?, "api_started");
    let result = http::serve(listener, router, shutdown.clone(), config.shutdown_timeout).await;
    shutdown.cancel();
    signals.abort();
    database.close().await;
    result?;
    tracing::info!("api_stopped");
    Ok(())
}
