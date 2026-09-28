use chrono::DateTime;
use nexofolio_intake_contracts::BatchLedger;
use nexofolio_observe_contracts::ObserveStore;
use std::{
    io,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
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

/// Housekeeping inside the api until shutdown: forgets processed collect
/// batches, in intake and in observe, once they are too old to be retried,
/// every hour and once right after start. Running it in several api
/// processes only deletes the same rows twice.
pub async fn run_housekeeping(
    shutdown: CancellationToken,
    ledger: Arc<dyn BatchLedger>,
    observe: Arc<dyn ObserveStore>,
) {
    let mut hourly = tokio::time::interval(Duration::from_secs(3600));
    hourly.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = async {
                hourly.tick().await;
                let now = DateTime::from(SystemTime::now());
                match nexofolio_intake::purge_expired(ledger.as_ref(), now).await {
                    Ok(purged) => tracing::info!(purged, "collect_batches_purged"),
                    Err(_) => tracing::warn!("collect_batch_purge_failed"),
                }
                match nexofolio_observe::purge_expired(observe.as_ref(), now).await {
                    Ok(purged) => tracing::info!(purged, "observed_batches_purged"),
                    Err(_) => tracing::warn!("observed_batch_purge_failed"),
                }
            } => {}
        }
    }
}
