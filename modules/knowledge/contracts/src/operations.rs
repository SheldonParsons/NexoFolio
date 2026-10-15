//! Small read projections for private operations; no scheduling or candidate payloads.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::TaskState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub project: Option<ProjectId>,
    pub state: Option<TaskState>,
    pub before: Option<TaskCursor>,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    pub run_id: Option<Uuid>,
    pub stage: Option<String>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub completed_batches: Option<u64>,
    /// Not collected by periodic reads. Exact batch metadata remains in on-demand diagnostics.
    pub saved_batch_records: Option<u64>,
    /// Last progress checkpoint timestamp, independent of candidate heartbeat updates.
    pub updated_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsTask {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub candidate_id: Option<Uuid>,
    pub target: Option<EndpointId>,
    pub state: TaskState,
    pub run_id: Option<Uuid>,
    pub generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub retry_reason: Option<String>,
    pub retry_count: i64,
    /// Present only for the current running owner. Checkpoint remains historical otherwise.
    pub stage: Option<String>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub checkpoint: Option<TaskCheckpoint>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPage {
    pub items: Vec<OperationsTask>,
    pub next: Option<TaskCursor>,
    pub observed_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCount {
    pub state: TaskState,
    pub count: i64,
}

#[async_trait]
pub trait OperationsReader: Send + Sync {
    async fn operations_tasks(&self, filter: &TaskFilter) -> super::StoreResult<TaskPage>;
    /// Background only. The adapter enforces a short read-only statement deadline.
    async fn operations_counts(&self) -> super::StoreResult<Vec<TaskCount>>;
    async fn task_project(&self, task: Uuid) -> super::StoreResult<Option<ProjectId>>;
}
