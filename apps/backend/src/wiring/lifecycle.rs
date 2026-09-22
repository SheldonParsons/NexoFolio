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
