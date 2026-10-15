//! Atomic waiting/attention transition, retry schedule and retained candidate checkpoint.
use super::{tasks::owned, validation::load};
use crate::store_error;
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge_contracts::KnowledgeTx;
pub(super) async fn suspend(
    tx: &mut dyn KnowledgeTx,
    task: &CurationTask,
    failure: TaskFailure,
    mut progress: serde_json::Value,
    now: chrono::DateTime<chrono::Utc>,
) -> KnowledgeResult<CurationTask> {
    let mut current = owned(tx, task, now).await?;
    let state = if failure.temporary {
        TaskState::Waiting
    } else {
        TaskState::NeedsAttention
    };
    current.transition(state, now);
    current.retry_count = current.retry_count.saturating_add(1);
    current.retry_reason = Some(failure.code);
    current.message = Some(failure.message);
    current.lease_until = None;
    if failure.temporary {
        // Shared provider admission is not another failed request. Escalating its
        // delay can repeatedly put healthy projects behind the failing project.
        let backoff = if current.retry_reason.as_deref() == Some("model_cooldown") {
            1
        } else {
            (5u64.saturating_mul(2u64.pow(current.retry_count.min(6)))).min(300)
        };
        let jitter = (uuid::Uuid::new_v4().as_u128() % (backoff / 5 + 1) as u128) as u64;
        let seconds = failure
            .retry_after_seconds
            .unwrap_or(0)
            .max(backoff)
            .saturating_add(jitter);
        let max = (chrono::DateTime::<chrono::Utc>::MAX_UTC - now).num_seconds() as u64;
        current.next_retry_at = Some(now + chrono::TimeDelta::seconds(seconds.min(max) as i64));
    }
    if let Some(id) = current.candidate_id {
        let mut candidate = load(tx, task.project_id, id).await?;
        if !progress.is_object() {
            progress = serde_json::json!({});
        }
        progress["run_id"] = serde_json::json!(task.run_id);
        progress["task_state"] = serde_json::json!(state);
        progress["status"] = serde_json::json!(if failure.temporary {
            "running"
        } else {
            "failed"
        });
        progress["active"] = serde_json::json!([]);
        progress["message"] = serde_json::json!(current.message);
        candidate.progress = progress;
        candidate.state = if failure.temporary && candidate.execution_input.is_none() {
            CandidateState::Running
        } else {
            CandidateState::Partial
        };
        candidate.updated_at = now;
        tx.put_candidate(&candidate).await.map_err(store_error)?;
    }
    tx.put_task(&current).await.map_err(store_error)?;
    Ok(current)
}
