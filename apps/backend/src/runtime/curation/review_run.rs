//! Proposal setup and durable progress. Official knowledge is only read here.
use super::*;
use nexofolio_contracts::knowledge::{CandidateState, KnowledgeError};
use nexofolio_knowledge::CandidateSession;

impl Curation {
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
        let live = self.progress(project);
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
        progress.candidate_state = Some(last.state);
        if last.state != CandidateState::Running
            && (progress.folders_saved.is_none() || progress.resume_available.is_none())
        {
            let detail = candidates.candidate(project, last.id).await?;
            progress.resume_available = Some(
                matches!(
                    last.state,
                    CandidateState::Partial | CandidateState::Interrupted
                ) && detail
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
            progress.message = Some("运行租约已过期；已保存批次保留，可补齐本轮缺口".into());
        }
        Ok(progress)
    }
}
