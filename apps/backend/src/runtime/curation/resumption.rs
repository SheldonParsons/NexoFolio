//! User resume and automatic recovery share the same frozen-input executor.
use super::{
    Curation, CurationError, Progress, Status,
    activity::Activity,
    jobs::{Attempt, Job},
    recovery::{FrozenInput, reconcile},
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{Candidate, CandidateState};
use std::sync::Arc;

pub(super) fn frozen(candidate: &Candidate) -> Result<FrozenInput, CurationError> {
    let value = candidate.execution_input.as_ref().ok_or_else(|| {
        CurationError::Conflict(
            "本轮未保存完整整理输入，无法安全续跑；请开始新一轮整理，已保存内容仍可查看".into(),
        )
    })?;
    let input: FrozenInput = serde::Deserialize::deserialize(value).map_err(|_| {
        CurationError::Conflict("本轮冻结资料格式不兼容，已保存内容保留；请开始新一轮整理".into())
    })?;
    if !matches!(input.version, 2 | 3)
        || (input.version == 3) != input.manifest.is_some()
        || (input.version == 2 && value.get("prepared").is_none())
        || input.manifest.as_ref().is_some_and(|m| {
            input.manifest_hash.as_deref()
                != Some(nexofolio_contracts::knowledge::frozen_input_hash(value).as_str())
                || m.project != candidate.project_id
                || m.candidate != candidate.id
        })
        || input.lifecycle_version != candidate.base.lifecycle.version
    {
        return Err(CurationError::Conflict(
            "本轮冻结资料版本不兼容，已保存内容保留；请开始新一轮整理".into(),
        ));
    }
    Ok(input)
}

pub(super) fn initialize(candidate: &Candidate, job: &Job, automatic: bool) {
    let mut p: Progress = serde_json::from_value(candidate.progress.clone()).unwrap_or_default();
    if automatic
        && let Some(old) = p
            .attempt_history
            .last_mut()
            .filter(|a| a.ended_at.is_none())
    {
        old.ended_at = Some(chrono::Utc::now());
        old.outcome = Some("interrupted".into());
    }
    p.run_id = Some(job.run_id);
    p.candidate_id = Some(candidate.id);
    p.lifecycle_version = candidate.base.lifecycle.version;
    p.lifecycle_stale = false;
    p.target = candidate.target;
    p.started_at.get_or_insert(candidate.created_at);
    if p.total == 0 {
        p.total = if candidate.target.is_some() {
            1
        } else {
            candidate.base.endpoints.len()
        };
    }
    p.status = Status::Running;
    p.failure_code = None;
    p.active.clear();
    p.heartbeat_at = None;
    p.resume_available = Some(true);
    p.recovery_blocked = false;
    p.message = automatic.then(|| "系统自动继续本轮整理".into());
    p.attempts += 1;
    p.attempt_started_at = Some(chrono::Utc::now());
    p.attempt_history.push(Attempt::start(job.run_id));
    let stage = p.stage.unwrap_or(nexofolio_curate::Pass::Describe);
    job.update(|progress| *progress = p);
    job.record(Activity::new(
        stage,
        "resume",
        if automatic {
            "系统自动继续本轮整理：沿用冻结资料，已确认保存的字段不重跑"
        } else {
            "补齐本轮缺口：沿用同一候选和冻结资料，已确认保存的字段不重跑"
        },
    ));
}

fn failed(job: &Job, error: &CurationError) {
    job.update(|p| {
        p.status = Status::Failed;
        p.active.clear();
        p.message = Some(error.to_string());
    });
}

impl Curation {
    pub(crate) async fn resume_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<Progress, CurationError> {
        if self.queue.is_some() {
            return self.enqueue(project, None, None, Some(id)).await;
        }
        if self.progress(project).status == Status::Running {
            return Err(CurationError::Busy);
        }
        let candidates = self.candidates.as_ref().ok_or(CurationError::Unavailable)?;
        let old = candidates.candidate(project, id).await?;
        if !matches!(
            old.state,
            CandidateState::Partial | CandidateState::Interrupted | CandidateState::Running
        ) {
            return Err(CurationError::Failed(
                "该候选已完成、应用或废弃，不能补写；请开始新一轮整理".into(),
            ));
        }
        let input = frozen(&old)?;
        let (job, created) = self.jobs.start(project, old.base.endpoints.len());
        if !created {
            return Err(CurationError::Busy);
        }
        let candidate = match candidates
            .resume_candidate_as(
                project,
                id,
                job.run_id,
                serde_json::to_value(&input).expect("frozen input"),
                self.actor.clone(),
            )
            .await
        {
            Ok(c) => c,
            Err(error) => {
                let error = error.into();
                failed(&job, &error);
                return Err(error);
            }
        };
        initialize(&candidate, &job, false);
        let result = self
            .finish_resume(project, candidate, input, job.clone())
            .await;
        if let Err(error) = &result {
            failed(&job, error);
        }
        result
    }

    pub(super) async fn recover_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<bool, CurationError> {
        if self.progress(project).status == Status::Running {
            return Ok(false);
        }
        let candidates = self.candidates.as_ref().ok_or(CurationError::Unavailable)?;
        let (job, created) = self.jobs.start(project, 0);
        if !created {
            return Ok(false);
        }
        let candidate = match candidates.recover_candidate(project, id, job.run_id).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                job.update(|p| p.status = Status::Idle);
                return Ok(false);
            }
            Err(error) => {
                let error = error.into();
                failed(&job, &error);
                return Err(error);
            }
        };
        initialize(&candidate, &job, true);
        let input = match frozen(&candidate) {
            Ok(input) => input,
            Err(error) => {
                failed(&job, &error);
                job.update(|p| {
                    p.resume_available = Some(false);
                    p.recovery_blocked = true;
                    if let Some(attempt) = p.attempt_history.last_mut() {
                        attempt.ended_at = Some(chrono::Utc::now());
                        attempt.outcome = Some("recovery_blocked".into());
                    }
                });
                self.checkpoint(project, &job, CandidateState::Interrupted)
                    .await?;
                tracing::warn!(%project,candidate=%id,"curation_auto_recovery_input_incompatible");
                return Ok(false);
            }
        };
        let result = self
            .finish_resume(project, candidate, input, job.clone())
            .await;
        if let Err(error) = &result {
            failed(&job, error);
        }
        result.map(|_| true)
    }

    pub(super) async fn finish_resume(
        &self,
        project: ProjectId,
        candidate: Candidate,
        input: FrozenInput,
        job: Arc<Job>,
    ) -> Result<Progress, CurationError> {
        job.update(|p| reconcile(&candidate, &input, p));
        let mut state = self.clone();
        let staged = Arc::new(nexofolio_knowledge::CandidateSession::new(
            &candidate,
            self.candidates
                .as_ref()
                .ok_or(CurationError::Unavailable)?
                .clone(),
        ));
        state.reader = staged.clone();
        state.knowledge = staged;
        state.resuming = true;
        let summaries = input.summaries.clone();
        state.frozen = Some(Arc::new(input));
        state
            .checkpoint(project, &job, CandidateState::Running)
            .await?;
        super::spawn(state, project, summaries, job.clone());
        Ok(job.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexofolio_common::EndpointId;
    #[test]
    fn single_interface_resume_keeps_its_scope_and_original_start() {
        let project = ProjectId::new();
        let endpoint = EndpointId::new();
        let started = chrono::Utc::now() - chrono::TimeDelta::hours(1);
        let mut candidate = Candidate {
            id: uuid::Uuid::new_v4(),
            project_id: project,
            target: Some(endpoint),
            parent: None,
            state: CandidateState::Running,
            created_at: started,
            updated_at: started,
            base_hash: "fixture".into(),
            base: nexofolio_contracts::knowledge::KnowledgeSnapshot {
                endpoints: vec![endpoint, EndpointId::new()],
                ..Default::default()
            },
            view: Default::default(),
            batches: vec![],
            progress: serde_json::json!({"total":1,"started_at":started,"fields_updated":7}),
            execution_input: None,
            applied_rounds: vec![],
            reviewed_by: None,
            operations: vec![],
        };
        let (job, _) = super::super::jobs::Jobs::default().start(project, 0);
        initialize(&candidate, &job, true);
        assert_eq!(job.snapshot().total, 1);
        assert_eq!(job.snapshot().started_at, Some(started));
        assert_eq!(job.snapshot().fields_updated, 7);
        candidate.progress = serde_json::json!({});
        initialize(&candidate, &job, true);
        assert_eq!(
            job.snapshot().total,
            1,
            "fallback must not turn a single interface into a project run"
        );
    }
}
