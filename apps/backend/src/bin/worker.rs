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
    let maintenance =
        nexofolio_backend::wiring::build_maintenance(&config, database.clone())?.map(|engine| {
            tokio::spawn(nexofolio_backend::wiring::run_maintenance_worker(
                engine,
                shutdown.clone(),
            ))
        });
    let evidence = if config.capture_enabled {
        let blobs = std::sync::Arc::new(nexofolio_infrastructure::FileBlobStore::new(
            config.blob_root.clone(),
        ));
        let store = nexofolio_infrastructure::PostgresCaptureStore::new(database.clone(), blobs);
        Some(tokio::spawn(
            nexofolio_backend::wiring::run_evidence_worker(store, shutdown.clone()),
        ))
    } else {
        None
    };
    if config.zentao_base_url.is_some() {
        let processor = std::sync::Arc::new(nexofolio_infrastructure::PostgresDocuments::new(
            database.clone(),
        ));
        nexofolio_backend::wiring::run_processing_worker(
            nexofolio_application::ProcessingService::new(processor),
            shutdown,
        )
        .await;
    } else {
        run_worker(shutdown).await;
    }
    if let Some(evidence) = evidence {
        let _ = evidence.await;
    }
    if let Some(maintenance) = maintenance {
        let _ = maintenance.await;
    }
    signals.abort();
    database.close().await;
    Ok(())
}
