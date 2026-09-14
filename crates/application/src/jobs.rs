use async_trait::async_trait;
use nexofolio_contracts::{JobId, ProjectId, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRequest {
    pub id: JobId,
    pub project_id: ProjectId,
    pub kind: String,
    pub input: Value,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOwnership {
    pub job_id: JobId,
    pub generation: u64,
    pub lease_expires_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub ownership: ExecutionOwnership,
    pub step: String,
    pub value: Value,
}

#[async_trait]
pub trait JobDispatcher: Send + Sync {
    async fn dispatch(&self, job: JobRequest) -> Result<()>;
}

#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Must reject a writer whose execution generation no longer owns the job.
    async fn save(&self, checkpoint: Checkpoint) -> Result<()>;
}

#[async_trait]
pub trait JobExecutor: Send + Sync {
    async fn execute(&self, job: JobRequest, ownership: ExecutionOwnership) -> Result<()>;
}
