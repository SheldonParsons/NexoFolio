//! In-memory ledger and the behaviour every [`BatchLedger`] must show.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::{BatchLedger, LedgerEntry, LedgerError, Receipt};

#[derive(Default)]
pub struct InMemoryLedger {
    entries: Mutex<HashMap<Uuid, LedgerEntry>>,
    unavailable: Mutex<bool>,
}

impl InMemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// While set, every call fails with [`LedgerError::Unavailable`].
    pub fn set_unavailable(&self, unavailable: bool) {
        *self.unavailable.lock().expect("fake lock") = unavailable;
    }

    pub fn len(&self) -> usize {
        self.entries.lock().expect("fake lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn check(&self) -> Result<(), LedgerError> {
        match *self.unavailable.lock().expect("fake lock") {
            true => Err(LedgerError::Unavailable),
            false => Ok(()),
        }
    }
}

#[async_trait]
impl BatchLedger for InMemoryLedger {
    async fn find(&self, batch_id: Uuid) -> Result<Option<LedgerEntry>, LedgerError> {
        self.check()?;
        Ok(self
            .entries
            .lock()
            .expect("fake lock")
            .get(&batch_id)
            .cloned())
    }

    async fn record(&self, entry: &LedgerEntry) -> Result<(), LedgerError> {
        self.check()?;
        self.entries
            .lock()
            .expect("fake lock")
            .entry(entry.batch_id)
            .or_insert_with(|| entry.clone());
        Ok(())
    }

    async fn purge(&self, before: DateTime<Utc>) -> Result<u64, LedgerError> {
        self.check()?;
        let mut entries = self.entries.lock().expect("fake lock");
        let count = entries.len();
        entries.retain(|_, entry| entry.recorded_at >= before);
        Ok((count - entries.len()) as u64)
    }
}

fn entry(recorded_at: DateTime<Utc>, hash: u8) -> LedgerEntry {
    let batch_id = Uuid::new_v4();
    LedgerEntry {
        batch_id,
        content_hash: [hash; 32],
        platform: "conformance".into(),
        receipt: Receipt {
            batch_id,
            accepted: 1,
            rejected: Vec::new(),
        },
        recorded_at,
    }
}

/// Uses fresh batch IDs, and timestamps far in the past so other data is
/// never purged. Panics on the first violation.
pub async fn batch_ledger_conformance<L: BatchLedger>(ledger: &L) {
    let long_ago = DateTime::from_timestamp(1_000_000_000, 0).expect("valid timestamp");
    let old = entry(long_ago, 1);
    let newer = entry(long_ago + TimeDelta::hours(1), 2);

    assert_eq!(ledger.find(old.batch_id).await, Ok(None), "unknown batch");
    ledger.record(&old).await.expect("record");
    ledger.record(&newer).await.expect("record second");
    assert_eq!(ledger.find(old.batch_id).await, Ok(Some(old.clone())));

    let mut other = old.clone();
    other.content_hash = [9; 32];
    other.receipt.accepted = 0;
    ledger
        .record(&other)
        .await
        .expect("second record is harmless");
    assert_eq!(
        ledger.find(old.batch_id).await,
        Ok(Some(old.clone())),
        "first entry wins"
    );

    let purged = ledger
        .purge(long_ago + TimeDelta::minutes(1))
        .await
        .expect("purge");
    assert_eq!(purged, 1, "only entries before the cutoff");
    assert_eq!(ledger.find(old.batch_id).await, Ok(None));
    assert_eq!(ledger.find(newer.batch_id).await, Ok(Some(newer)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_batch_ledger_conformance() {
        batch_ledger_conformance(&InMemoryLedger::new()).await;
    }
}
