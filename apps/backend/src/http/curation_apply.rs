//! Bounded process-local observations. The candidate's persisted state is authoritative.
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{ApplyObserver, ApplyProgress, ApplyStage};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;
type Key = (ProjectId, Uuid, Uuid);
#[derive(Clone, Default)]
pub(super) struct ApplyObservations(Arc<Mutex<HashMap<Key, Entry>>>);
struct Entry {
    at: Instant,
    progress: ApplyProgress,
    active: bool,
}
pub(super) struct Observation {
    entries: ApplyObservations,
    key: Key,
}
impl ApplyObservations {
    pub fn start(
        &self,
        project: ProjectId,
        candidate: Uuid,
        operation: Uuid,
    ) -> Option<Observation> {
        let mut entries = self.0.lock().ok()?;
        entries.retain(|_, e| e.active || e.at.elapsed() < Duration::from_secs(600));
        // Never replay an operation identity, including after an uncertain response.
        if entries.contains_key(&(project, candidate, operation)) || entries.len() >= 256 {
            return None;
        }
        entries.insert(
            (project, candidate, operation),
            Entry {
                at: Instant::now(),
                active: true,
                progress: ApplyProgress {
                    stage: ApplyStage::Waiting,
                    processed: 0,
                    total: 0,
                },
            },
        );
        Some(Observation {
            entries: self.clone(),
            key: (project, candidate, operation),
        })
    }
    pub fn get(
        &self,
        project: ProjectId,
        candidate: Uuid,
        operation: Uuid,
    ) -> Option<ApplyProgress> {
        self.0
            .lock()
            .ok()?
            .get(&(project, candidate, operation))
            .filter(|e| e.active || e.at.elapsed() < Duration::from_secs(600))
            .map(|e| e.progress.clone())
    }
}
impl ApplyObserver for Observation {
    fn update(&self, progress: ApplyProgress) {
        if let Ok(mut entries) = self.entries.0.lock() {
            if let Some(entry) = entries.get_mut(&self.key) {
                entry.active = !matches!(
                    progress.stage,
                    ApplyStage::Committed | ApplyStage::Failed | ApplyStage::Unknown
                );
                entry.at = Instant::now();
                entry.progress = progress;
            }
        }
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.entries.0.lock() {
            if let Some(entry) = entries.get_mut(&self.key) {
                if entry.active {
                    entry.progress.stage = ApplyStage::Unknown;
                    entry.active = false;
                    entry.at = Instant::now();
                }
            }
        }
    }
}
