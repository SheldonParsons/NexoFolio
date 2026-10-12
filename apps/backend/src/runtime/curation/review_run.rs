//! Proposal setup and durable progress. Official knowledge is only read here.
use super::*;
use nexofolio_contracts::knowledge::{CandidateState, KnowledgeError};
use nexofolio_knowledge::CandidateSession;

impl Curation {
    pub(crate) async fn start_target(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
    ) -> Result<Progress, CurationError> {
        let summaries = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| CurationError::Unavailable)?;
        if target.is_some_and(|id| {
            !summaries
                .iter()
                .any(|e| e.id == id && e.project_id == project)
        }) {
            return Err(CurationError::NotFound);
        }
        let existing = self.jobs.snapshot(project);
        if existing.status == Status::Running && existing.target != target {
            return Err(CurationError::Busy);
        }
        let (job, created) = self
            .jobs
            .start(project, if target.is_some() { 1 } else { summaries.len() });
        if created {
            job.update(|p| p.target = target);
            let state = match self
                .stage_run(project, target, parent, &summaries, &job)
                .await
            {
                Ok(state) => state,
                Err(error) => {
                    job.update(|p| {
                        p.status = Status::Failed;
                        p.message = Some(error.to_string());
                    });
                    return Err(error);
                }
            };
            let lifecycle = state.reader.lifecycle(project).await?;
            let summaries: Vec<_> = summaries
                .into_iter()
                .filter(|e| lifecycle.endpoint(e.id).is_none())
                .collect();
            job.update(|p| p.total = if target.is_some() { 1 } else { summaries.len() });
            spawn(state, project, summaries, job.clone());
        }
        Ok(job.snapshot())
    }

    pub(crate) fn with_candidates(mut self, candidates: Arc<dyn KnowledgeCandidates>) -> Self {
        self.candidates = Some(candidates);
        self
    }
    pub(super) async fn stage_run(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        summaries: &[nexofolio_contracts::endpoint::EndpointSummary],
        job: &Job,
    ) -> Result<Self, CurationError> {
        let Some(candidates) = &self.candidates else {
            return Ok(self.clone());
        };
        // New runs start from official knowledge. Draft inheritance is explicit only;
        // continuing a stopped draft uses resume_candidate and keeps its identity/input.
        let mut candidate = match candidates
            .create_candidate_as(project, target, parent, self.actor.clone())
            .await
        {
            Err(KnowledgeError::CandidateStale) if parent.is_some() => {
                candidates
                    .create_candidate_as(project, target, None, self.actor.clone())
                    .await?
            }
            other => other?,
        };
        job.update(|p| {
            p.candidate_id = Some(candidate.id);
            p.lifecycle_version = candidate.base.lifecycle.version;
            if candidate.parent.is_some() && candidate.batches.iter().any(|b| matches!(&b.author,nexofolio_contracts::knowledge::Author::Curate { pass,.. } if pass == "organize")) {
                p.directory_input_hash = Some(nexofolio_curate::directory_input_hash(summaries,&candidate.view.notes,&candidate.view.folders,&candidate.view.placements));
            }
        });
        self.checkpoint(project, job, CandidateState::Running)
            .await?;
        candidate.progress = serde_json::to_value(job.snapshot()).expect("progress");
        let staged = Arc::new(CandidateSession::new(&candidate, candidates.clone()));
        let mut state = self.clone();
        state.reader = staged.clone();
        state.knowledge = staged;
        state
            .checkpoint(project, job, CandidateState::Running)
            .await?;
        Ok(state)
    }
    pub(super) async fn checkpoint(
        &self,
        project: ProjectId,
        job: &Job,
        state: CandidateState,
    ) -> Result<(), CurationError> {
        job.update(|p| p.candidate_state = Some(state));
        if let (Some(candidates), Some(id)) = (&self.candidates, job.snapshot().candidate_id) {
            candidates
                .checkpoint_candidate(
                    project,
                    id,
                    state,
                    serde_json::to_value(job.snapshot()).expect("progress"),
                )
                .await?;
        }
        Ok(())
    }
    pub(crate) async fn saved_progress(
        &self,
        project: ProjectId,
    ) -> Result<Progress, CurationError> {
        let mut live = self.progress(project);
        let lifecycle_version = self.reader.lifecycle(project).await?.version;
        live.lifecycle_stale =
            live.candidate_id.is_some() && live.lifecycle_version != lifecycle_version;
        if live.status == Status::Running {
            return Ok(live);
        }
        let Some(candidates) = &self.candidates else {
            return Ok(live);
        };
        let latest = match candidates.active_candidate(project).await? {
            Some(active) => Some(active),
            None => candidates
                .candidates(project, 50)
                .await?
                .into_iter()
                .max_by_key(|c| c.updated_at),
        };
        let Some(last) = latest else {
            return Ok(live);
        };
        let mut progress: Progress = serde_json::from_value(last.progress).unwrap_or_default();
        progress.candidate_id = Some(last.id);
        progress.lifecycle_stale = progress.lifecycle_version != lifecycle_version;
        if progress.lifecycle_stale {
            progress.resume_available = Some(false);
        }
        progress.candidate_state = Some(last.state);
        if last.state != CandidateState::Running
            && (progress.folders_saved.is_none() || progress.resume_available.is_none())
        {
            let detail = candidates.candidate(project, last.id).await?;
            progress.resume_available = Some(
                matches!(
                    last.state,
                    CandidateState::Partial | CandidateState::Interrupted
                ) && !progress.recovery_blocked
                    && detail
                        .execution_input
                        .as_ref()
                        .is_some_and(|input| input["version"] == 2),
            );
            progress.folders_saved = Some(detail.view.folders.len());
            // The old executor returns this specific field error only after directory success.
            if last.target.is_none()
                && (last.state == CandidateState::Ready
                    || (progress.stage == Some(nexofolio_curate::Pass::Organize)
                        && progress.fields_failed > 0
                        && progress
                            .message
                            .as_deref()
                            .is_some_and(|m| m.contains("个接口有字段未完成"))))
            {
                progress.directory_completed = true;
            }
        }
        if last.state == CandidateState::Running
            && chrono::Utc::now() - last.updated_at > chrono::TimeDelta::seconds(120)
        {
            progress.status = Status::Failed;
            progress.active.clear();
            progress.message = Some(if progress.resume_available == Some(true) {
                "原执行进程已停止响应；已保存批次保留，系统正在尝试自动续跑本轮".into()
            } else {
                "原执行进程已停止响应，本轮没有兼容的恢复资料；已保存内容保留，请开始新一轮整理"
                    .into()
            });
        }
        if progress.lifecycle_stale {
            progress.resume_available = Some(false);
        }
        Ok(progress)
    }
}
