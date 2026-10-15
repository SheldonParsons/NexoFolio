//! Durable curation admission; candidate content remains in the existing lifecycle.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Waiting,
    NeedsAttention,
    Done,
    Failed,
    Stopped,
}
impl TaskState {
    pub fn unfinished(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Waiting | Self::NeedsAttention
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurationTask {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub target: Option<EndpointId>,
    pub parent: Option<uuid::Uuid>,
    pub candidate_id: Option<uuid::Uuid>,
    pub actor: CandidateActor,
    pub state: TaskState,
    pub generation: i64,
    pub run_id: Option<uuid::Uuid>,
    pub lease_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_started_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub retry_reason: Option<String>,
    pub retry_count: u32,
    pub timing: TaskTiming,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskTiming {
    pub queued_ms: i64,
    pub waiting_ms: i64,
    pub running_ms: i64,
    pub since: Option<DateTime<Utc>>,
}
impl CurationTask {
    pub fn elapsed(&self, now: DateTime<Utc>) -> TaskTiming {
        let mut timing = self.timing.clone();
        let elapsed = (now - timing.since.unwrap_or(self.updated_at))
            .num_milliseconds()
            .max(0);
        match self.state {
            TaskState::Queued => timing.queued_ms = timing.queued_ms.saturating_add(elapsed),
            TaskState::Waiting => timing.waiting_ms = timing.waiting_ms.saturating_add(elapsed),
            TaskState::Running => timing.running_ms = timing.running_ms.saturating_add(elapsed),
            _ => {}
        }
        timing.since = Some(now);
        timing
    }
    pub fn transition(&mut self, state: TaskState, now: DateTime<Utc>) {
        self.timing = self.elapsed(now);
        self.timing.since = Some(now);
        self.state = state;
        self.updated_at = now;
        if state != TaskState::Waiting {
            self.next_retry_at = None;
        }
    }
}

/// Backend task operations. No credentials, model policy or MCP tools here.
#[async_trait]
pub trait KnowledgeTasks: Send + Sync {
    async fn submit_task(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        resume: Option<uuid::Uuid>,
        actor: CandidateActor,
    ) -> KnowledgeResult<CurationTask>;
    async fn task(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<CurationTask>;
    async fn task_for_candidate(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> KnowledgeResult<Option<CurationTask>>;
    async fn active_tasks(&self, project: ProjectId) -> KnowledgeResult<Vec<CurationTask>>;
    async fn claim_task(&self, capacity: usize) -> KnowledgeResult<Option<CurationTask>>;
    async fn prepare_task(&self, task: &CurationTask) -> KnowledgeResult<Candidate>;
    async fn heartbeat_task(&self, task: &CurationTask) -> KnowledgeResult<()>;
    async fn finish_task(
        &self,
        task: &CurationTask,
        state: TaskState,
        message: Option<String>,
    ) -> KnowledgeResult<()>;
    async fn suspend_task(
        &self,
        task: &CurationTask,
        failure: TaskFailure,
        progress: serde_json::Value,
    ) -> KnowledgeResult<CurationTask>;
    async fn stop_task(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<()>;
}

/// Safe reason and retry policy from the executor; scheduling stays transactional.
#[derive(Debug, Clone)]
pub struct TaskFailure {
    pub code: String,
    pub message: String,
    pub temporary: bool,
    pub retry_after_seconds: Option<u64>,
}

/// Allowlisted persisted reason; free-form messages never enter private exports.
pub fn safe_task_reason(code: Option<&str>) -> Option<&str> {
    code.filter(|code| {
        matches!(
            *code,
            "model_configuration"
                | "model_network"
                | "model_timeout"
                | "model_rate_limited"
                | "model_unavailable"
                | "model_rejected"
                | "model_cooldown"
                | "invalid_reply"
                | "RESOURCE_NOT_FOUND"
                | "SERVICE_UNAVAILABLE"
                | "CURATION_NOT_SUBMITTED"
                | "CANDIDATE_CONFLICT"
                | "PASS_FAILED"
                | "PROJECT_BUSY"
        )
    })
}
