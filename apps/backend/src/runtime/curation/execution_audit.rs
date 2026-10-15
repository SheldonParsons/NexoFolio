//! Reconstruct execution evidence from durable task identity using the existing bounded sink.
use super::{Curation, jobs::Job};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::TaskState;
impl Curation {
    pub(crate) fn with_execution_audit(mut self, audit: crate::audit::Audit) -> Self {
        self.execution_audit = Some(audit);
        self
    }
    pub(super) async fn record_execution_end(
        &self,
        project: ProjectId,
        job: &Job,
        default_code: &str,
        success: bool,
        duration_ms: u32,
    ) {
        let Some(audit) = &self.audit else { return };
        let mut code = default_code;
        if let (Some(queue), Some(task)) = (&self.queue, &self.task)
            && let Ok(current) = queue.tasks.task(project, task.id).await
        {
            if current.generation != task.generation || current.run_id != task.run_id {
                code = "LEASE_LOST";
            } else if current.state == TaskState::Stopped {
                code = "STOPPED";
            }
        }
        audit.record(
            if success || code == "STOPPED" {
                "operation"
            } else {
                "error"
            },
            "curation.finished",
            code,
            job.snapshot().candidate_id.map(|id| id.to_string()),
            duration_ms,
        );
    }
}
