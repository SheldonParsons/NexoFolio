//! Durable admission feeds the existing frozen-input executor; no second worker engine.
use super::{Curation, CurationError, Progress, Status};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::{CurationTask, KnowledgeTasks, TaskState};
use std::sync::Arc;

pub(super) struct Queue {
    pub tasks: Arc<dyn KnowledgeTasks>,
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    instance: String,
    capacity: usize,
}
impl Queue {
    pub(super) async fn authorize(&self, task: &CurationTask) -> Result<(), CurationError> {
        let principal = SessionPrincipal {
            user_id: task.actor.user_id().ok_or_else(|| {
                CurationError::Conflict("无法核验旧任务发起者，请授权用户手动续跑".into())
            })?,
            instance: self.instance.clone(),
        };
        // Reuse enabled-account and current project membership gates, without a login token.
        self.sessions.me(&principal).await.map_err(access_error)?;
        self.projects
            .require_project(&principal, task.project_id)
            .await
            .map_err(access_error)?;
        Ok(())
    }
}
fn access_error(error: nexofolio_common::Error) -> CurationError {
    match error {
        nexofolio_common::Error::Unavailable { .. } => CurationError::Unavailable,
        _ => CurationError::Conflict(
            "发起者账户或项目权限已不可用，任务已停止；已保存批次保留".into(),
        ),
    }
}
impl Curation {
    pub(crate) fn with_queue(
        mut self,
        tasks: Arc<dyn KnowledgeTasks>,
        sessions: Arc<dyn Sessions>,
        projects: Arc<dyn ProjectAccess>,
        instance: String,
        capacity: usize,
    ) -> Self {
        self.queue = Some(Arc::new(Queue {
            tasks,
            sessions,
            projects,
            instance,
            capacity: capacity.max(1),
        }));
        self
    }
    pub(super) async fn enqueue(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        resume: Option<uuid::Uuid>,
    ) -> Result<Progress, CurationError> {
        let task = self
            .queue
            .as_ref()
            .ok_or(CurationError::Unavailable)?
            .tasks
            .submit_task(project, target, parent, resume, self.actor.clone())
            .await
            .map_err(|e| {
                if matches!(
                    e,
                    nexofolio_contracts::knowledge::KnowledgeError::Unavailable
                ) {
                    CurationError::NotSubmitted
                } else {
                    e.into()
                }
            })?;
        if let Some(audit) = &self.audit {
            audit.associate_task(&task);
        }
        // No fallible post-commit reads: the receipt always reflects durable acceptance.
        Ok(receipt(&task))
    }
    pub(crate) async fn active_tasks(
        &self,
        project: ProjectId,
    ) -> Result<Vec<CurationTask>, CurationError> {
        self.queue
            .as_ref()
            .ok_or(CurationError::Unavailable)?
            .tasks
            .active_tasks(project)
            .await
            .map_err(Into::into)
    }
    pub(crate) async fn task_by_id(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<CurationTask, CurationError> {
        self.queue
            .as_ref()
            .ok_or(CurationError::Unavailable)?
            .tasks
            .task(project, id)
            .await
            .map_err(Into::into)
    }
    pub(crate) async fn stop_task(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<(), CurationError> {
        let live = self.progress(project);
        self.queue
            .as_ref()
            .ok_or(CurationError::Unavailable)?
            .tasks
            .stop_task(project, id)
            .await?;
        if live.task_id == Some(id) {
            self.jobs.cancel_run(project, live.run_id);
        }
        Ok(())
    }
    pub(crate) async fn task_progress(
        &self,
        task: &CurationTask,
    ) -> Result<Progress, CurationError> {
        let live = self.progress(task.project_id);
        let mut p = if task.state == TaskState::Running
            && live.run_id == task.run_id
            && live.task_id == Some(task.id)
        {
            live
        } else if let (Some(candidates), Some(id)) = (&self.candidates, task.candidate_id) {
            let candidate = candidates
                .candidate_diagnostics(task.project_id, id)
                .await?
                .summary;
            let mut p: Progress = serde_json::from_value(candidate.progress).unwrap_or_default();
            p.candidate_id = Some(id);
            p.candidate_state = Some(candidate.state);
            p
        } else {
            receipt(task)
        };
        p.task_id = Some(task.id);
        p.task_state = Some(task.state);
        p.next_retry_at = task.next_retry_at;
        p.retry_reason = task.retry_reason.clone();
        p.retry_count = task.retry_count;
        p.task_timing = Some(task.elapsed(chrono::Utc::now()));
        if task.state == TaskState::Running {
            p.status = Status::Running;
            if p.run_id != task.run_id {
                p.active.clear();
                p.stage = None;
                p.message = Some("正在领取并恢复任务".into());
            }
            p.run_id = task.run_id;
        }
        if task.state == TaskState::Queued {
            p.status = Status::Running;
            p.stage = None;
            p.active.clear();
            p.message = Some("任务已接受，正在排队；关闭页面不影响执行".into());
        }
        if matches!(task.state, TaskState::Waiting | TaskState::NeedsAttention) {
            p.status = if task.state == TaskState::Waiting {
                Status::Running
            } else {
                Status::Failed
            };
            p.stage = None;
            p.active.clear();
            p.message = task.message.clone();
            p.resume_available = Some(
                task.state == TaskState::NeedsAttention
                    && p.resume_available == Some(true)
                    && task.retry_reason.as_deref() != Some("CANDIDATE_CONFLICT"),
            );
        }
        if !task.state.unfinished() {
            p.status = if task.state == TaskState::Done {
                Status::Done
            } else {
                Status::Failed
            };
            p.message = task.message.clone().or(p.message);
            p.active.clear();
        }
        let version = self.reader.lifecycle(task.project_id).await?.version;
        p.lifecycle_stale = p.candidate_id.is_some() && p.lifecycle_version != version;
        if p.lifecycle_stale {
            p.resume_available = Some(false);
        }
        Ok(p)
    }
    pub(super) async fn dispatch(&self) -> Result<(), CurationError> {
        let queue = self.queue.as_ref().ok_or(CurationError::Unavailable)?;
        for _ in 0..queue.capacity {
            let Some(task) = queue.tasks.claim_task(queue.capacity).await? else {
                break;
            };
            let mut state = self.clone();
            state.actor = task.actor.clone();
            state.task = Some(task.clone());
            state.audit = self
                .execution_audit
                .clone()
                .map(|audit| crate::audit::Context::execution(audit, &task));
            let (job, _) = state.jobs.start_owned(task.project_id, 0, task.run_id);
            tokio::spawn(async move {
                job.update(|p| {
                    p.task_id = Some(task.id);
                    p.task_state = Some(TaskState::Running);
                    p.target = task.target;
                });
                let queue = state.queue.as_ref().expect("queue");
                let setup = async {
                    queue.authorize(&task).await?;
                    let candidate = queue.tasks.prepare_task(&task).await?;
                    if candidate.operations.iter().any(|o| {
                        o.action == nexofolio_contracts::knowledge::CandidateAction::Resume
                    }) {
                        let automatic = candidate.operations.last().is_some_and(|o| {
                            matches!(
                                o.actor,
                                nexofolio_contracts::knowledge::CandidateActor::System { .. }
                            )
                        });
                        super::resumption::initialize(&candidate, &job, automatic);
                        if automatic && let Some(audit) = &state.audit {
                            audit.record(
                                "operation",
                                if task.retry_count > 0 {
                                    "curation.resumed"
                                } else {
                                    "curation.recovered"
                                },
                                if task.retry_count > 0 {
                                    "RETRY_DUE"
                                } else {
                                    "LEASE_TAKEOVER"
                                },
                                Some(candidate.id.to_string()),
                                0,
                            );
                        }
                    } else {
                        job.update(|p| {
                            p.candidate_id = Some(candidate.id);
                            p.lifecycle_version = candidate.base.lifecycle.version;
                            p.total = if candidate.target.is_some() {
                                1
                            } else {
                                candidate.base.endpoints.len()
                            };
                        });
                    }
                    job.update(|p| {
                        p.task_id = Some(task.id);
                        p.task_state = Some(TaskState::Running);
                    });
                    if candidate.execution_input.is_some() {
                        let input = super::resumption::frozen(&candidate)?;
                        state
                            .finish_resume(task.project_id, candidate, input, job.clone())
                            .await?;
                    } else {
                        let summaries = state
                            .endpoints
                            .list(task.project_id)
                            .await
                            .map_err(|_| CurationError::Unavailable)?;
                        let lifecycle = state.reader.lifecycle(task.project_id).await?;
                        let summaries: Vec<_> = summaries
                            .into_iter()
                            .filter(|e| lifecycle.endpoint(e.id).is_none())
                            .collect();
                        job.update(|p| {
                            p.total=if candidate.target.is_some(){1}else{summaries.len()};
                            if candidate.parent.is_some() && candidate.batches.iter().any(|b|matches!(&b.author,nexofolio_contracts::knowledge::Author::Curate{pass,..} if pass=="organize")) {
                                p.directory_input_hash=Some(nexofolio_curate::directory_input_hash(&summaries,&candidate.view.notes,&candidate.view.folders,&candidate.view.placements));
                            }
                        });
                        let staged = Arc::new(nexofolio_knowledge::CandidateSession::new(
                            &candidate,
                            state
                                .candidates
                                .as_ref()
                                .ok_or(CurationError::Unavailable)?
                                .clone(),
                        ));
                        let mut worker = state.clone();
                        worker.reader = staged.clone();
                        worker.knowledge = staged;
                        worker
                            .checkpoint(
                                task.project_id,
                                &job,
                                nexofolio_contracts::knowledge::CandidateState::Running,
                            )
                            .await?;
                        super::spawn(worker, task.project_id, summaries, job.clone());
                    }
                    Ok(())
                };
                if let Err(error) = super::supervision::supervise(
                    &job,
                    setup,
                    super::supervision::heartbeat(&state, task.project_id, &job),
                    state.idle_budget,
                )
                .await
                {
                    if state.defer(&job, &error).await {
                        return;
                    }
                    job.update(|p| {
                        p.status = Status::Failed;
                        p.message = Some(error.to_string());
                    });
                    let code = error.diagnostic_code();
                    if let Err(save) = queue
                        .tasks
                        .finish_task(&task, TaskState::Failed, Some(error.to_string()))
                        .await
                    {
                        tracing::warn!(%save,task=%task.id,"curation_task_setup_failed");
                    }
                    state
                        .record_execution_end(task.project_id, &job, code, false, 0)
                        .await;
                }
            });
        }
        Ok(())
    }
}
fn receipt(task: &CurationTask) -> Progress {
    Progress {
        task_id: Some(task.id),
        task_state: Some(task.state),
        target: task.target,
        candidate_id: task.candidate_id,
        run_id: task.run_id,
        status: if task.state.unfinished() {
            Status::Running
        } else if task.state == TaskState::Done {
            Status::Done
        } else {
            Status::Failed
        },
        stage: None,
        message: Some(if task.state == TaskState::Queued {
            "任务已接受，正在排队；关闭页面不影响执行".into()
        } else {
            task.message.clone().unwrap_or_else(|| "任务已接受".into())
        }),
        ..Default::default()
    }
}
