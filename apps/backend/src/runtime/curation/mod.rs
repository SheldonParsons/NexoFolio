//! Application execution composes ports. Rules and pass dependencies live in curate.
mod activity;
#[cfg(test)]
mod candidate_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod description_retry_tests;
mod descriptions;
mod directories;
#[cfg(test)]
mod efficiency_tests;
mod error;
mod field_scheduler;
mod field_status;
mod field_worker;
mod fields;
mod jobs;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod model_benchmark_tests;
#[cfg(test)]
mod model_effort_benchmark_tests;
#[cfg(test)]
mod model_stage_benchmark_tests;
#[cfg(test)]
mod performance_snapshot_tests;
mod preparation;
mod preparation_progress;
#[cfg(test)]
mod readonly_resume_tests;
mod recovery;
mod restart;
#[cfg(test)]
mod restart_tests;
#[cfg(test)]
mod resume_tests;
mod resumption;
mod review_run;
#[cfg(test)]
mod semantic_retry_tests;
mod snapshot;
mod stages;
mod standard_pass;
mod supervision;
#[cfg(test)]
mod supervision_tests;
#[cfg(test)]
mod tests;

pub(crate) use error::CurationError;
use jobs::{Job, Jobs};
pub(crate) use jobs::{Progress, Status};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{
    endpoint::{EndpointReader, EndpointSummary},
    knowledge::{
        CandidateActor, CandidateState, KnowledgeCandidates, KnowledgeReader, KnowledgeWriter,
    },
};
use nexofolio_curate::{Completions, Describe, Organize, Pass, workflow};
use std::sync::Arc;
use tracing::Instrument;

#[derive(Clone)]
pub(crate) struct Curation {
    knowledge: Arc<dyn KnowledgeWriter>,
    reader: Arc<dyn KnowledgeReader>,
    endpoints: Arc<dyn EndpointReader>,
    describe: Arc<Describe<Arc<dyn Completions>>>,
    organize: Arc<Organize<Arc<dyn Completions>>>,
    jobs: Arc<Jobs>,
    candidates: Option<Arc<dyn KnowledgeCandidates>>,
    field_concurrency: usize,
    idle_budget: std::time::Duration,
    description_concurrency: usize,
    frozen: Option<Arc<recovery::FrozenInput>>,
    resuming: bool,
    actor: CandidateActor,
    audit: Option<crate::audit::Context>,
}
impl Curation {
    pub(crate) fn new(
        knowledge: Arc<dyn KnowledgeWriter>,
        reader: Arc<dyn KnowledgeReader>,
        endpoints: Arc<dyn EndpointReader>,
        describe: Arc<Describe<Arc<dyn Completions>>>,
        organize: Arc<Organize<Arc<dyn Completions>>>,
    ) -> Self {
        Self {
            knowledge,
            reader,
            endpoints,
            describe,
            organize,
            jobs: Arc::new(Jobs::default()),
            candidates: None,
            field_concurrency: 8,
            idle_budget: std::time::Duration::from_secs(900),
            description_concurrency: 4,
            frozen: None,
            resuming: false,
            actor: CandidateActor::Unknown,
            audit: None,
        }
    }

    pub(crate) fn with_concurrency(mut self, fields: usize, descriptions: usize) -> Self {
        self.field_concurrency = fields.clamp(1, 16);
        self.description_concurrency = descriptions.clamp(1, 8);
        self
    }
    pub(crate) fn with_idle_timeout(mut self, idle: std::time::Duration) -> Self {
        self.idle_budget = idle;
        self
    }
    pub(crate) fn with_audit(mut self, audit: Option<crate::audit::Context>) -> Self {
        self.audit = audit;
        self
    }
    pub(crate) fn with_actor(mut self, actor: CandidateActor) -> Self {
        self.actor = actor;
        self
    }

    pub(crate) fn progress(&self, project: ProjectId) -> Progress {
        self.jobs.snapshot(project)
    }

    pub(crate) fn forget_project(&self, project: ProjectId, expected: Option<uuid::Uuid>) {
        self.jobs.forget(project, expected);
    }
    pub(crate) fn has_running_project(&self, project: ProjectId) -> bool {
        self.jobs.snapshot(project).status == Status::Running
    }

    pub(crate) async fn start(&self, project: ProjectId) -> Result<Progress, CurationError> {
        self.start_target(project, None, None).await
    }
}

fn spawn(s: Curation, project: ProjectId, summaries: Vec<EndpointSummary>, job: Arc<Job>) {
    // The observer turns a worker panic into a failed run instead of a stuck spinner.
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        if let Some(audit) = &s.audit {
            audit.record(
                "operation",
                "curation.running",
                "RUNNING",
                job.snapshot().candidate_id.map(|id| id.to_string()),
                0,
            );
        }
        let progress = job.clone();
        let checkpoint = s.clone();
        let worker = tokio::spawn(async move {
            recovery::execute(&s, project, &summaries, &progress)
                .instrument(
                    tracing::info_span!("curation", project = %project, run = %progress.run_id),
                )
                .await
        });
        let candidate_state = match worker.await {
            Ok(Ok(())) => {
                job.update(|p| {
                    p.status = Status::Done;
                    p.stage = None;
                });
                CandidateState::Ready
            }
            Ok(Err(error)) => {
                let failure_code = error.diagnostic_code().to_owned();
                let state = if matches!(error, CurationError::Unavailable) {
                    CandidateState::Interrupted
                } else {
                    CandidateState::Partial
                };
                job.update(|p| {
                    p.status = Status::Failed;
                    p.active.clear();
                    p.failure_code = Some(failure_code);
                    p.message = Some(match error {
                        CurationError::Failed(message) => message,
                        _ => "整理暂时无法完成，已保存的候选内容保留，可补跑。".into(),
                    });
                });
                state
            }
            Err(error) => {
                job.update(|p| {
                    p.failure_code = Some(
                        if error.is_panic() {
                            "WORKER_PANIC"
                        } else {
                            "WORKER_CANCELLED"
                        }
                        .into(),
                    );
                    p.status = Status::Failed;
                    p.active.clear();
                    p.message = Some("整理被中断，已保存批次保留，可补跑。".into());
                });
                CandidateState::Interrupted
            }
        };
        job.update(|p| {
            if let Some(attempt) = p
                .attempt_history
                .iter_mut()
                .find(|a| a.run_id == job.run_id)
            {
                attempt.ended_at = Some(chrono::Utc::now());
                attempt.outcome = Some(
                    if candidate_state == CandidateState::Ready {
                        "ready"
                    } else {
                        "partial"
                    }
                    .into(),
                );
            }
        });
        let snapshot = job.snapshot();
        job.record(activity::Activity::new(
            snapshot.stage.unwrap_or(Pass::Organize),
            if candidate_state == CandidateState::Ready {
                "complete"
            } else {
                "partial"
            },
            snapshot
                .message
                .unwrap_or_else(|| "本轮候选已保存，等待你的审阅".into()),
        ));
        if checkpoint
            .checkpoint(project, &job, candidate_state)
            .await
            .is_err()
        {
            job.update(|p| {
                p.status = Status::Failed;
                p.message = Some("结果保存暂时失败，已保存批次保留，请刷新或补跑".into());
            });
        }
        if let Some(audit) = &checkpoint.audit {
            let progress = job.snapshot();
            let successful = progress.status == Status::Done;
            let code = if successful {
                "READY"
            } else if candidate_state == CandidateState::Interrupted {
                "INTERRUPTED"
            } else {
                "PARTIAL_OR_SAVE_FAILED"
            };
            audit.record(
                if successful { "operation" } else { "error" },
                "curation.finished",
                code,
                progress.candidate_id.map(|id| id.to_string()),
                started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            );
        }
    });
}

async fn run(
    s: &Curation,
    project: ProjectId,
    summaries: &[EndpointSummary],
    job: &Job,
) -> Result<(), CurationError> {
    tracing::info!(endpoints = summaries.len(), "curation started");
    let mut prepared = recovery::prepared(s, project, summaries, job).await?;
    let target = job.snapshot().target;
    if let Some(target) = target {
        prepared.facts.retain(|f| f.summary.id == target);
    }
    job.update(|p| p.message = None);
    let mut field_failure = None;
    for pass in workflow() {
        if pass == Pass::Organize
            && (target.is_some() || (s.resuming && job.snapshot().directory_completed))
        {
            continue;
        }
        job.update(|p| p.stage = Some(pass));
        job.record(activity::Activity::new(
            pass,
            "stage",
            match pass {
                Pass::Describe => "正在核对接口名称与用途，复用未变化的说明",
                Pass::Fields => "正在核对字段证据，只更新需要补充的内容",
                Pass::Organize => "正在生成业务模块与功能分组的两层目录",
            },
        ));
        s.checkpoint(project, job, CandidateState::Running).await?;
        let result = stages::run(s, project, summaries, job, target, &prepared, pass).await;
        if result.is_err() {
            tracing::warn!(stage = ?pass, "curation stage failed");
        }
        // Incomplete field meanings must not prevent browsing the known APIs.
        // A storage failure remains fatal; model failures leave explicit gaps.
        match result {
            Err(error @ CurationError::Failed(_)) if pass == Pass::Fields => {
                if job.snapshot().issues.iter().any(|i| !i.retryable) {
                    return Err(error);
                }
                field_failure = Some(error);
            }
            other => other?,
        }
        if pass == Pass::Organize && target.is_none() {
            job.update(|p| p.directory_completed = true);
            job.record(activity::Activity::new(pass, "saved", "两层目录候选已保存"));
        }
    }
    // Final coverage check reads the acknowledged projection; only still-missing inputs are retried.
    for sweep in 0..2 {
        field_status::refresh(s, &prepared, job).await?;
        let (missing, incomplete_endpoints) = fields::coverage(s, &prepared).await?;
        job.update(|p| p.fields_failed = incomplete_endpoints);
        if missing == 0 {
            field_failure = None;
            break;
        }
        job.update(|p| {
            p.recovery_sweeps += 1;
            p.stage = Some(Pass::Fields);
        });
        job.record(activity::Activity::new(
            Pass::Fields,
            "coverage_retry",
            format!(
                "收尾复查发现缺口，正在本轮补跑（{}/2）；已保存字段不重跑",
                sweep + 1
            ),
        ));
        tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(sweep + 1))).await;
        match fields::run(s, project, job, target, &prepared).await {
            Ok(()) => field_failure = None,
            Err(error @ CurationError::Failed(_)) => {
                if job.snapshot().issues.iter().any(|i| !i.retryable) {
                    return Err(error);
                }
                field_failure = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    field_status::refresh(s, &prepared, job).await?;
    let (missing, incomplete_endpoints) = fields::coverage(s, &prepared).await?;
    job.update(|p| p.fields_failed = incomplete_endpoints);
    if missing > 0 && field_failure.is_none() {
        field_failure = Some(CurationError::Failed(format!(
            "收尾复查仍有 {missing} 个字段未完成；补跑预算已用完，已保存批次保留"
        )));
    }
    tracing::info!(
        generated = job.snapshot().generated,
        fields_updated = job.snapshot().fields_updated,
        "curation completed"
    );
    field_failure.map_or(Ok(()), Err)
}
