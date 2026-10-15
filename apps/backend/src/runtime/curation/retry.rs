//! Persist one classified execution exit; the existing queue owns all future attempts.
use super::{Curation, CurationError, Status};
impl Curation {
    pub(super) async fn defer(&self, job: &super::jobs::Job, error: &CurationError) -> bool {
        let (Some(queue), Some(task)) = (&self.queue, &self.task) else {
            return false;
        };
        let temporary = matches!(error, CurationError::Unavailable)
            || matches!(error, CurationError::Model(error) if error.temporary());
        let retry_after_seconds = match error {
            CurationError::Model(error) => error.retry_after_seconds(),
            _ => None,
        };
        let failure = nexofolio_contracts::knowledge::TaskFailure {
            code: error.diagnostic_code().into(),
            message: if temporary {
                format!("{error}；等待恢复后自动继续，已保存成果保留")
            } else {
                format!("需要处理：{error}；修复后可继续本轮，或停止任务")
            },
            temporary,
            retry_after_seconds,
        };
        job.update(|p| {
            p.failure_code = Some(error.diagnostic_code().into());
            if let Some(attempt) = p
                .attempt_history
                .iter_mut()
                .find(|a| a.run_id == job.run_id)
            {
                attempt.ended_at = Some(chrono::Utc::now());
                attempt.outcome = Some(
                    if temporary {
                        "waiting"
                    } else {
                        "needs_attention"
                    }
                    .into(),
                );
            }
        });
        match queue
            .tasks
            .suspend_task(
                task,
                failure,
                serde_json::to_value(job.snapshot()).expect("progress"),
            )
            .await
        {
            Ok(saved) => {
                job.update(|p| {
                    p.task_state = Some(saved.state);
                    p.status = if temporary {
                        Status::Running
                    } else {
                        Status::Failed
                    };
                    p.message = saved.message;
                    p.active.clear();
                    p.stage = None;
                });
                if let Some(audit) = &self.audit {
                    audit.record(
                        "operation",
                        "curation.suspended",
                        error.diagnostic_code(),
                        Some(saved.candidate_id.unwrap_or(task.id).to_string()),
                        0,
                    );
                }
            }
            Err(save) => {
                // Never claim that an unacknowledged transition was saved. Stop local IO;
                // the same durable lease will be recovered when storage becomes available.
                tracing::warn!(%save,task=%task.id,"curation_suspend_not_saved");
                job.update(|p| {
                    p.status = Status::Failed;
                    p.active.clear();
                    p.message =
                        Some("执行已退出，等待核验持久任务状态；已确认保存的批次保留".into());
                });
            }
        }
        self.record_execution_end(task.project_id, job, error.diagnostic_code(), false, 0)
            .await;
        true
    }
}
