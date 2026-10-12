//! Periodic recovery of abandoned workers; no new queue or business scheduling.
use super::{Curation, CurationError};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const PAGE: usize = 20;
impl Curation {
    pub(crate) async fn recover_after_restart(self, shutdown: CancellationToken) {
        let mut ticker = tokio::time::interval(Duration::from_secs(20));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _=shutdown.cancelled()=>break,
                _=ticker.tick()=>{}
            }
            tokio::select! {
                biased;
                _=shutdown.cancelled()=>break,
                result=self.recover_expired()=>{
                    if let Err(error)=result {
                        tracing::warn!(%error,"curation_auto_recovery_scan_failed");
                    }
                }
            }
        }
    }
    pub(super) async fn recover_expired(&self) -> Result<(), CurationError> {
        let Some(candidates) = &self.candidates else {
            return Ok(());
        };
        let mut after = None;
        loop {
            let rows = candidates.recovery_candidates(after, PAGE).await?;
            for row in &rows {
                match self.recover_candidate(row.project_id, row.id).await {
                    Ok(true) => {
                        tracing::info!(project=%row.project_id,candidate=%row.id,"curation_auto_recovered")
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(project=%row.project_id,candidate=%row.id,%error,"curation_auto_recovery_failed")
                    }
                }
            }
            if rows.len() < PAGE {
                break;
            }
            after = rows.last().map(|row| row.id);
        }
        Ok(())
    }
}
