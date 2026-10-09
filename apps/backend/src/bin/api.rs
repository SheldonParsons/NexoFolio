use nexofolio_backend::{
    http,
    wiring::{
        Config, Databases, build_api, build_navigation, init_logging, install_shutdown_handler,
        run_housekeeping,
    },
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let databases = Databases::new(&config)?;
    let listener = TcpListener::bind(config.bind_addr).await?;
    let shutdown = CancellationToken::new();
    let signals = install_shutdown_handler(shutdown.clone())?;
    let api = build_api(&config, &databases)?;
    let router = http::router_with_navigation(
        &config,
        Arc::new(databases.access.clone()),
        Arc::new(databases.access.clone()),
        shutdown.clone(),
        api,
        Some(build_navigation(&databases)),
    );
    tracing::info!(address = %listener.local_addr()?, "api_started");
    let housekeeping = tokio::spawn(run_housekeeping(
        shutdown.clone(),
        Arc::new(databases.intake.clone()),
        Arc::new(databases.observe.clone()),
    ));
    let result = http::serve(listener, router, shutdown.clone(), config.shutdown_timeout).await;
    shutdown.cancel();
    let _ = housekeeping.await;
    signals.abort();
    databases.close().await;
    result?;
    tracing::info!("api_stopped");
    Ok(())
}
