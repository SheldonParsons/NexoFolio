//! Private maintenance port; the adapter owns policy and transaction safety.
use chrono::{DateTime, Utc};
use nexofolio_common::ProjectId;
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Debug, Default, Serialize)]
pub struct RetentionReport {
    pub committed: bool,
    pub scanned: usize,
    pub reference_records: i64,
    pub deferred_objects: BTreeMap<uuid::Uuid, String>,
    pub reference_bytes: i64,
    pub compacted: usize,
    pub inputs_released: usize,
    pub archives_reclaimed: usize,
    pub chunks_deleted: u64,
    pub grace_started: usize,
    pub skipped: BTreeMap<String, usize>,
    pub next: Option<uuid::Uuid>,
}
impl RetentionReport {
    pub fn skip(&mut self, reason: &str) {
        *self.skipped.entry(reason.into()).or_default() += 1;
    }
}
#[derive(Clone, Serialize)]
pub struct RetentionStatus {
    pub enabled_at: Option<DateTime<Utc>>,
    pub protected_until: Option<DateTime<Utc>>,
    /// Stopped tasks whose candidate is still recoverable. Bounded, not a total.
    pub stopped_recoverable: Vec<uuid::Uuid>,
    pub more_stopped: bool,
}
#[async_trait::async_trait]
pub trait HistoryMaintenance: Send + Sync {
    async fn retention_status(&self, project: ProjectId) -> super::StoreResult<RetentionStatus>;
    async fn maintain(
        &self,
        project: ProjectId,
        after: Option<uuid::Uuid>,
        execute: bool,
    ) -> super::StoreResult<RetentionReport>;
}
