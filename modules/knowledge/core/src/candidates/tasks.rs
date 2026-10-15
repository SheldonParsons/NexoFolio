//! Admission and ownership use the same project transaction as candidate writes.
use super::{recovery, snapshot, validation::load};
use crate::{Clock, Knowledge, store_error};
use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::{KnowledgeStore, KnowledgeTx};

pub(super) async fn owned(
    tx: &mut dyn KnowledgeTx,
    expected: &CurationTask,
    now: chrono::DateTime<chrono::Utc>,
) -> KnowledgeResult<CurationTask> {
    let task = tx
        .task(expected.project_id, expected.id)
        .await
        .map_err(store_error)?
        .ok_or(KnowledgeError::NoSuchCandidate)?;
    if task.state != TaskState::Running
        || task.generation != expected.generation
        || task.run_id != expected.run_id
        || task.lease_until.is_none_or(|until| until <= now)
    {
        return Err(KnowledgeError::CandidateClosed);
    }
    Ok(task)
}
pub(super) async fn worker(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    candidate: uuid::Uuid,
    owner: Option<uuid::Uuid>,
    now: chrono::DateTime<chrono::Utc>,
) -> KnowledgeResult<()> {
    if !tx
        .valid_worker(project, candidate, owner, now)
        .await
        .map_err(store_error)?
    {
        return Err(KnowledgeError::CandidateClosed);
    }
    Ok(())
}
#[async_trait]
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> KnowledgeTasks for Knowledge<S, R, C> {
    async fn submit_task(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        resume: Option<uuid::Uuid>,
        actor: CandidateActor,
    ) -> KnowledgeResult<CurationTask> {
        let target = if let Some(id) = target {
            let identity = self
                .endpoints
                .identity(id)
                .await
                .map_err(|_| KnowledgeError::Unavailable)?
                .ok_or(KnowledgeError::WrongProject)?;
            if identity.project_id != project {
                return Err(KnowledgeError::WrongProject);
            }
            Some(identity.id)
        } else {
            None
        };
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut previous = None;
        let target = if let Some(id) = resume {
            let candidate = tx
                .candidate_diagnostics(project, id)
                .await
                .map_err(store_error)?
                .ok_or(KnowledgeError::NoSuchCandidate)?
                .summary;
            if !matches!(
                candidate.state,
                CandidateState::Running | CandidateState::Partial | CandidateState::Interrupted
            ) {
                return Err(KnowledgeError::CandidateClosed);
            }
            previous = tx.candidate_task(project, id).await.map_err(store_error)?;
            candidate.target
        } else {
            target
        };
        // Identity is project + canonical scope. The first outstanding request wins,
        // including its explicit parent/resume intent and initiator.
        if let Some(existing) = tx
            .active_tasks(project)
            .await
            .map_err(store_error)?
            .into_iter()
            .find(|t| t.target == target)
        {
            if existing.state == TaskState::NeedsAttention
                && ((resume.is_some() && resume == existing.candidate_id)
                    || existing.candidate_id.is_none())
            {
                previous = Some(existing);
            } else {
                return Ok(existing);
            }
        }
        if let Some(id) = target {
            let lifecycle = tx.lifecycle(project).await.map_err(store_error)?;
            if lifecycle.endpoint(id).is_some() {
                return Err(KnowledgeError::InvalidLifecycle("请先撤销接口废弃".into()));
            }
        }
        if let Some(parent) = parent {
            tx.candidate_state(project, parent)
                .await
                .map_err(store_error)?
                .ok_or(KnowledgeError::NoSuchCandidate)?;
        }
        let now = self.clock.now();
        let mut task = previous.unwrap_or(CurationTask {
            id: uuid::Uuid::new_v4(),
            project_id: project,
            target,
            parent,
            candidate_id: resume,
            actor: actor.clone(),
            state: TaskState::Queued,
            generation: 0,
            run_id: None,
            lease_until: None,
            created_at: now,
            updated_at: now,
            last_started_at: None,
            message: None,
            next_retry_at: None,
            retry_reason: None,
            retry_count: 0,
            timing: Default::default(),
        });
        task.transition(TaskState::Queued, now);
        task.retry_reason = None;
        task.retry_count = 0;
        task.actor = actor;
        task.lease_until = None;
        task.updated_at = now;
        task.message = None;
        tx.put_task(&task).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(task)
    }
    async fn task(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<CurationTask> {
        self.store
            .begin_read()
            .await
            .map_err(store_error)?
            .task(project, id)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)
    }
    async fn task_for_candidate(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> KnowledgeResult<Option<CurationTask>> {
        self.store
            .begin_read()
            .await
            .map_err(store_error)?
            .candidate_task(project, candidate)
            .await
            .map_err(store_error)
    }
    async fn active_tasks(&self, project: ProjectId) -> KnowledgeResult<Vec<CurationTask>> {
        self.store
            .begin_read()
            .await
            .map_err(store_error)?
            .active_tasks(project)
            .await
            .map_err(store_error)
    }
    async fn claim_task(&self, capacity: usize) -> KnowledgeResult<Option<CurationTask>> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        let now = self.clock.now();
        let Some(mut task) = tx.next_task(now, capacity).await.map_err(store_error)? else {
            return Ok(None);
        };
        let now = self.clock.now();
        task.transition(TaskState::Running, now);
        task.generation += 1;
        task.run_id = Some(uuid::Uuid::new_v4());
        task.updated_at = now;
        task.last_started_at = Some(now);
        task.lease_until = Some(now + chrono::TimeDelta::seconds(recovery::LEASE_SECONDS));
        tx.put_task(&task).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(task))
    }
    async fn prepare_task(&self, task: &CurationTask) -> KnowledgeResult<Candidate> {
        if task.candidate_id.is_none() {
            let prepared = self
                .create_proposal_for_task(
                    task.project_id,
                    task.target,
                    task.parent,
                    task.actor.clone(),
                    Some(task),
                )
                .await;
            return match prepared {
                Err(KnowledgeError::CandidateStale) if task.parent.is_some() => {
                    self.create_proposal_for_task(
                        task.project_id,
                        task.target,
                        None,
                        task.actor.clone(),
                        Some(task),
                    )
                    .await
                }
                result => result,
            };
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(task.project_id)
            .await
            .map_err(store_error)?;
        let current = owned(tx.as_mut(), task, self.clock.now()).await?;
        let mut candidate = load(
            tx.as_mut(),
            task.project_id,
            current
                .candidate_id
                .ok_or(KnowledgeError::NoSuchCandidate)?,
        )
        .await?;
        if !matches!(
            candidate.state,
            CandidateState::Running | CandidateState::Partial | CandidateState::Interrupted
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        let endpoints = self
            .endpoints
            .list(task.project_id)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let base = snapshot::capture(tx.as_mut(), task.project_id, endpoints).await?;
        if base.lifecycle.version != candidate.base.lifecycle.version {
            return Err(KnowledgeError::LifecycleStale);
        }
        if snapshot::hash(&base) != candidate.base_hash
            || (candidate.execution_input.is_none()
                && (!candidate.batches.is_empty() || candidate.state != CandidateState::Running))
        {
            return Err(KnowledgeError::CandidateStale);
        }
        let actor = if candidate.state == CandidateState::Running || task.retry_count > 0 {
            CandidateActor::System {
                id: "curation-recovery".into(),
                display_name: "系统".into(),
            }
        } else {
            task.actor.clone()
        };
        recovery::claim(
            &mut candidate,
            task.run_id.ok_or(KnowledgeError::CandidateClosed)?,
            actor,
            self.clock.now(),
        );
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(candidate)
    }
    async fn heartbeat_task(&self, task: &CurationTask) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(task.project_id)
            .await
            .map_err(store_error)?;
        let now = self.clock.now();
        let mut current = owned(tx.as_mut(), task, now).await?;
        current.lease_until = Some(now + chrono::TimeDelta::seconds(recovery::LEASE_SECONDS));
        current.updated_at = now;
        tx.put_task(&current).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    async fn finish_task(
        &self,
        task: &CurationTask,
        state: TaskState,
        message: Option<String>,
    ) -> KnowledgeResult<()> {
        if state.unfinished() {
            return Err(KnowledgeError::CandidateClosed);
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(task.project_id)
            .await
            .map_err(store_error)?;
        let mut current = owned(tx.as_mut(), task, self.clock.now()).await?;
        interrupt(tx.as_mut(), &current, self.clock.now(), state).await?;
        current.transition(state, self.clock.now());
        current.message = message;
        current.lease_until = None;
        current.updated_at = self.clock.now();
        tx.put_task(&current).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    async fn suspend_task(
        &self,
        task: &CurationTask,
        failure: TaskFailure,
        progress: serde_json::Value,
    ) -> KnowledgeResult<CurationTask> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(task.project_id)
            .await
            .map_err(store_error)?;
        let now = self.clock.now();
        let current = super::task_retry::suspend(tx.as_mut(), task, failure, progress, now).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(current)
    }
    async fn stop_task(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut task = tx
            .task(project, id)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)?;
        if task.state.unfinished() {
            interrupt(tx.as_mut(), &task, self.clock.now(), TaskState::Stopped).await?;
            task.transition(TaskState::Stopped, self.clock.now());
            task.lease_until = None;
            task.updated_at = self.clock.now();
            task.message = Some("用户已停止任务；已保存批次保留".into());
            tx.put_task(&task).await.map_err(store_error)?;
        }
        tx.commit().await.map_err(store_error)
    }
}
async fn interrupt(
    tx: &mut dyn KnowledgeTx,
    task: &CurationTask,
    now: chrono::DateTime<chrono::Utc>,
    state: TaskState,
) -> KnowledgeResult<()> {
    if let Some(id) = task.candidate_id
        && matches!(
            tx.candidate_state(task.project_id, id)
                .await
                .map_err(store_error)?,
            Some(CandidateState::Running | CandidateState::Partial)
        )
    {
        let mut candidate = load(tx, task.project_id, id).await?;
        if candidate.state == CandidateState::Running {
            candidate.state = CandidateState::Interrupted;
        }
        candidate.updated_at = now;
        candidate.progress["status"] = serde_json::json!("failed");
        candidate.progress["task_state"] = serde_json::json!(state);
        candidate.progress["active"] = serde_json::json!([]);
        candidate.progress["message"] = serde_json::json!("任务已停止；已保存批次保留");
        tx.put_candidate(&candidate).await.map_err(store_error)?;
    }
    Ok(())
}
