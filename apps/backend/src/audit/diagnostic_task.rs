//! Safe current task evidence; candidate checkpoints cannot supply a retry schedule.
use nexofolio_contracts::knowledge::{CurationTask, TaskState};
use serde_json::{Value, json};

pub(super) fn status(state: TaskState) -> &'static str {
    match state {
        TaskState::Queued => "queued",
        TaskState::Running => "running",
        TaskState::Waiting => "waiting",
        TaskState::NeedsAttention => "needs_attention",
        TaskState::Done => "review",
        TaskState::Failed => "failed",
        TaskState::Stopped => "stopped",
    }
}
pub(super) fn evidence(t: &CurationTask, now: chrono::DateTime<chrono::Utc>) -> Value {
    let reason = nexofolio_contracts::knowledge::safe_task_reason(t.retry_reason.as_deref());
    json!({"id":t.id,"project_id":t.project_id,"candidate_id":t.candidate_id,"target":t.target,
        "state":t.state,"run_id":t.run_id,"generation":t.generation,"lease_until":t.lease_until,
        "created_at":t.created_at,"updated_at":t.updated_at,"observed_at":now,
        "next_retry_at":t.next_retry_at,"retry_reason":reason,"retry_count":t.retry_count,
        "timing":t.timing.since.map(|_|t.elapsed(now)),"timing_basis":"persisted_state_counters; pre-upgrade phase totals were not reconstructed"})
}
