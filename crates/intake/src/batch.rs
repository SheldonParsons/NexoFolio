//! Versioned external envelope; per-kind payload validation lives at the HTTP boundary.
use async_trait::async_trait;
use nexofolio_contracts::{ProjectId, Result, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub r#type: String,
    pub instance_id: Uuid,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionBatch {
    pub schema_version: String,
    pub batch_id: Uuid,
    pub project_id: ProjectId,
    pub environment: nexofolio_contracts::EnvironmentRef,
    /// Only for replaying immutable v1 envelopes; never a dedup partition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_key: Option<String>,
    pub source: Producer,
    pub records: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Accepted,
    Ignored,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordReceipt {
    pub record_index: usize,
    pub record_id: Option<Uuid>,
    pub status: ReceiptStatus,
    pub reason_code: String,
    pub retryable: bool,
    pub ingestion_id: Option<Uuid>,
    pub replayed: bool,
}
impl RecordReceipt {
    pub fn reject(index: usize, id: Option<Uuid>, code: &str) -> Self {
        Self {
            record_index: index,
            record_id: id,
            status: ReceiptStatus::Rejected,
            reason_code: code.into(),
            retryable: false,
            ingestion_id: None,
            replayed: false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchReceipt {
    pub schema_version: String,
    pub batch_id: Uuid,
    pub results: Vec<RecordReceipt>,
    pub environment: Option<nexofolio_contracts::Environment>,
}
/// Database-independent preparation. No payload values may be printed to logs.
pub struct PreparedRecord {
    pub index: usize,
    pub record_id: Uuid,
    pub content_hash: Vec<u8>,
    pub identity_key: String,
    pub path_identity: Option<nexofolio_contracts::PathIdentity>,
    pub structural_hash: Option<Vec<u8>>,
    pub structural_projection: Option<Value>,
    pub raw: Value,
}
pub struct Admission {
    pub actor: UserId,
    pub instance: String,
    pub project_id: ProjectId,
    pub batch_id: Uuid,
    pub environment: nexofolio_contracts::EnvironmentRef,
    pub legacy_service_key: Option<String>,
    pub source: Producer,
    pub records: Vec<PreparedRecord>,
}
#[async_trait]
pub trait AdmissionStore: Send + Sync {
    /// One batch transaction: recheck access, dedup, persist downstream input and receipts.
    async fn admit(&self, batch: Admission) -> Result<AdmissionResult>;
}

pub struct AdmissionResult {
    pub environment: nexofolio_contracts::Environment,
    pub receipts: Vec<RecordReceipt>,
}

#[async_trait]
pub trait ProjectPathPolicies: Send + Sync {
    async fn read(&self, project: ProjectId) -> Result<nexofolio_contracts::PathPolicy>;
    async fn set(&self, project: ProjectId, policy: &nexofolio_contracts::PathPolicy)
    -> Result<()>;
}

pub struct PreparedCapture {
    pub index: usize,
    pub record: nexofolio_contracts::CaptureRecord,
    pub raw: Value,
    pub content_hash: Vec<u8>,
    pub http: Option<PreparedRecord>,
}
#[async_trait]
pub trait CaptureAdmissionStore: Send + Sync {
    async fn admit_captures(
        &self,
        actor: UserId,
        instance: &str,
        batch: nexofolio_contracts::CaptureBatch,
        records: Vec<PreparedCapture>,
    ) -> Result<nexofolio_contracts::CaptureBatchReceipt>;
}
