//! What intake core and its storage adapter share: the receipt a batch gets
//! and the ledger that remembers it.
//!
//! The public collect contract is the JSON in `contracts/collect/v1`; these
//! types only mirror its receipt so the ledger can store and replay it.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "testing")]
pub mod testing;

/// Response body of an accepted batch (`receipt.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub batch_id: Uuid,
    pub accepted: usize,
    pub rejected: Vec<Rejection>,
}

/// One record that was not accepted. Every other record was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejection {
    /// Position in `records`.
    pub index: usize,
    /// The record's `id` as sent, empty when it had none.
    pub id: String,
    pub reason: RejectReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RejectReason {
    InvalidRecord,
    UnsupportedKind,
    UnsupportedVersion,
    RecordTooLarge,
}

impl RejectReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRecord => "INVALID_RECORD",
            Self::UnsupportedKind => "UNSUPPORTED_KIND",
            Self::UnsupportedVersion => "UNSUPPORTED_VERSION",
            Self::RecordTooLarge => "RECORD_TOO_LARGE",
        }
    }
}

/// A processed batch, kept for a while so retries get the same receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub batch_id: Uuid,
    /// SHA-256 of the batch as canonical JSON.
    pub content_hash: [u8; 32],
    pub platform: String,
    pub receipt: Receipt,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// Retryable: storage is down.
    #[error("batch ledger unavailable")]
    Unavailable,
}

/// Remembers processed batches. Implemented by the intake adapter.
#[async_trait]
pub trait BatchLedger: Send + Sync {
    async fn find(&self, batch_id: Uuid) -> Result<Option<LedgerEntry>, LedgerError>;

    /// The first entry for a batch wins; recording it again changes nothing.
    async fn record(&self, entry: &LedgerEntry) -> Result<(), LedgerError>;

    /// Forgets entries recorded before `before`; returns how many.
    async fn purge(&self, before: DateTime<Utc>) -> Result<u64, LedgerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_serialise_like_the_contract() {
        let receipt = Receipt {
            batch_id: Uuid::nil(),
            accepted: 1,
            rejected: vec![Rejection {
                index: 1,
                id: "r".into(),
                reason: RejectReason::UnsupportedKind,
                message: None,
            }],
        };
        let value = serde_json::to_value(&receipt).unwrap();
        assert_eq!(value["rejected"][0]["reason"], "UNSUPPORTED_KIND");
        assert!(value["rejected"][0].get("message").is_none());
        assert_eq!(serde_json::from_value::<Receipt>(value).unwrap(), receipt);
        assert_eq!(RejectReason::RecordTooLarge.code(), "RECORD_TOO_LARGE");
    }
}
