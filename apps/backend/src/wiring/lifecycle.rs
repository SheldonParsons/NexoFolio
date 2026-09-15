use std::io;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub fn install_shutdown_handler(shutdown: CancellationToken) -> io::Result<JoinHandle<()>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        Ok(tokio::spawn(async move {
            tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
            tracing::info!("shutdown_requested");
            shutdown.cancel();
        }))
    }
    #[cfg(not(unix))]
    {
        Ok(tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.cancel();
        }))
    }
}

/// Foundation lifecycle only. No in-memory queue pretending to be durable work.
pub async fn run_worker(shutdown: CancellationToken) {
    tracing::info!(mode = "idle", "worker_started_no_job_source_configured");
    shutdown.cancelled().await;
    tracing::info!("worker_stopped");
}

/// Durable inbox worker. Cancellation leaves an uncommitted claim recoverable by lease expiry.
pub async fn run_processing_worker(
    service: nexofolio_application::ProcessingService,
    shutdown: CancellationToken,
) {
    tracing::info!("document_worker_started");
    loop {
        let result = tokio::select! {
         _=shutdown.cancelled()=>break,
         result=tokio::time::timeout(std::time::Duration::from_secs(90),service.process_one())=>result,
        };
        match result {
            Ok(Ok(Some(result))) => {
                tracing::info!(ingestion_id=%result.ingestion_id,interface_id=%result.interface_id,outcome=%result.outcome,"observation_processed");
                continue;
            }
            Ok(Ok(None)) => {}
            _ => tracing::warn!("document_processing_deferred_or_failed"),
        }
        tokio::select! {_=shutdown.cancelled()=>break,_=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}}
    }
    tracing::info!("document_worker_stopped");
}

pub async fn run_evidence_worker(
    store: nexofolio_infrastructure::PostgresCaptureStore,
    shutdown: CancellationToken,
) {
    tracing::info!("evidence_worker_started");
    let mut last_gc = std::time::Instant::now();
    loop {
        if last_gc.elapsed() > std::time::Duration::from_secs(60) {
            let result = tokio::select! {
                _ = shutdown.cancelled() => break,
                result = tokio::time::timeout(std::time::Duration::from_secs(30), store.collect_expired_evidence()) => result,
            };
            if !matches!(result, Ok(Ok(_))) {
                tracing::warn!("evidence_retention_deferred");
            }
            last_gc = std::time::Instant::now();
        }
        let result =
            tokio::select! {_=shutdown.cancelled()=>break,r=store.process_evidence_one()=>r};
        if matches!(result, Ok(true)) {
            continue;
        }
        if result.is_err() {
            tracing::warn!("evidence_processing_deferred");
        }
        tokio::select! {_=shutdown.cancelled()=>break,_=tokio::time::sleep(std::time::Duration::from_millis(500))=>{}}
    }
    tracing::info!("evidence_worker_stopped");
}

pub async fn run_maintenance_worker(
    engine: nexofolio_application::MaintenanceEngine,
    shutdown: CancellationToken,
) {
    tracing::info!("maintenance_worker_started");
    loop {
        let result = tokio::select! {_=shutdown.cancelled()=>break,r=engine.tick()=>r};
        if matches!(result, Ok(true)) {
            continue;
        }
        if result.is_err() {
            tracing::warn!("maintenance_worker_deferred")
        }
        tokio::select! {_=shutdown.cancelled()=>break,_=tokio::time::sleep(std::time::Duration::from_secs(2))=>{}}
    }
    tracing::info!("maintenance_worker_stopped");
}
