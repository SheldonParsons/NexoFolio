//! Execution snapshots, same-ID resume and lease-aware cancellation. No semantic rules or SQL.
use super::{Curation, CurationError, Progress, Status, jobs::Job, preparation::PreparedProject};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::{
    endpoint::EndpointSummary,
    knowledge::{Candidate, CandidateState, Command},
};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FrozenInput {
    version: u32,
    pub summaries: Vec<EndpointSummary>,
    pub prepared: PreparedProject,
    pub baseline_batches: Vec<RoundId>,
    pub legacy: bool,
}
impl Curation {
    pub(crate) async fn resume_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<Progress, CurationError> {
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
        let frozen = match &old.execution_input {
            Some(value) => serde_json::from_value::<FrozenInput>(value.clone()).map_err(|_| {
                CurationError::Failed("本轮冻结资料格式不兼容，请开始新一轮整理".into())
            })?,
            None => {
                return Err(CurationError::Conflict(
                    "本轮未保存完整整理输入，无法安全续跑；请开始新一轮整理，已保存内容仍可查看"
                        .into(),
                ));
            }
        };
        if frozen.version != 2 {
            return Err(CurationError::Failed(
                "本轮冻结资料版本不兼容，请开始新一轮整理".into(),
            ));
        }
        let (job, created) = self.jobs.start(project, old.base.endpoints.len());
        if !created {
            return Err(CurationError::Busy);
        }
        let candidate = match candidates
            .resume_candidate_as(
                project,
                id,
                job.run_id,
                serde_json::to_value(&frozen).expect("frozen input"),
                self.actor.clone(),
            )
            .await
        {
            Ok(candidate) => candidate,
            Err(error) => {
                job.update(|p| {
                    p.status = Status::Failed;
                    p.message = Some(error.to_string());
                });
                return Err(error.into());
            }
        };
        let mut progress: Progress =
            serde_json::from_value(candidate.progress.clone()).unwrap_or_default();
        reconcile(&candidate, &frozen, &mut progress);
        progress.run_id = Some(job.run_id);
        progress.candidate_id = Some(id);
        progress.target = candidate.target;
        progress.status = Status::Running;
        progress.active.clear();
        progress.heartbeat_at = None;
        progress.resume_available = Some(true);
        progress.message = None;
        progress.attempts += 1;
        progress.attempt_started_at = Some(chrono::Utc::now());
        progress
            .attempt_history
            .push(super::jobs::Attempt::start(job.run_id));
        job.update(|p| *p = progress);
        job.record(super::activity::Activity::new(
            nexofolio_curate::Pass::Fields,
            "resume",
            "补齐本轮缺口：沿用同一候选和冻结资料，已确认保存的字段不重跑",
        ));
        let mut state = self.clone();
        let staged = Arc::new(nexofolio_knowledge::CandidateSession::new(
            &candidate,
            candidates.clone(),
        ));
        state.reader = staged.clone();
        state.knowledge = staged;
        state.resuming = true;
        let summaries = frozen.summaries.clone();
        state.frozen = Some(Arc::new(frozen));
        state
            .checkpoint(project, &job, CandidateState::Running)
            .await?;
        super::spawn(state, project, summaries, job.clone());
        Ok(job.snapshot())
    }
    pub(crate) async fn cancel(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<(), CurationError> {
        // Release the local worker's transaction before cancellation waits for its lock.
        if self.progress(project).candidate_id == Some(id) {
            self.jobs.cancel(project);
        }
        let candidates = self.candidates.as_ref().ok_or(CurationError::Unavailable)?;
        match candidates.cancel_candidate(project, id).await {
            Ok(()) => {}
            Err(nexofolio_contracts::knowledge::KnowledgeError::CandidateClosed)
                if candidates.candidate(project, id).await?.state != CandidateState::Running => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}
fn reconcile(candidate: &Candidate, frozen: &FrozenInput, p: &mut Progress) {
    let mut fields = HashSet::new();
    let mut descriptions = HashSet::new();
    for batch in candidate
        .batches
        .iter()
        .filter(|b| !frozen.baseline_batches.contains(&b.id))
    {
        if matches!(&batch.author, nexofolio_contracts::knowledge::Author::Curate { pass, .. } if pass == "organize")
            && !batch.commands.is_empty()
            && (!frozen.legacy || candidate.parent.is_none())
        {
            p.directory_completed = true;
            p.folders_saved = Some(candidate.view.folders.len());
        }
        for command in &batch.commands {
            match command {
                Command::PutFieldNote { endpoint, path, .. } => {
                    fields.insert((*endpoint, path));
                }
                Command::PutEndpointNote { endpoint, .. } => {
                    descriptions.insert(*endpoint);
                }
                _ => {}
            }
        }
    }
    p.saved_field_keys = fields
        .iter()
        .map(|(endpoint, path)| format!("{endpoint}/{path}"))
        .collect();
    p.fields_updated = fields.len();
    p.generated = descriptions.len();
}
pub(super) async fn prepared(
    s: &Curation,
    project: ProjectId,
    summaries: &[EndpointSummary],
    job: &Job,
) -> Result<PreparedProject, CurationError> {
    if let Some(input) = &s.frozen {
        return Ok(input.prepared.clone());
    }
    use super::preparation_progress::PreparationPhase;
    job.prepare_start(summaries.len());
    s.checkpoint(project, job, CandidateState::Running).await?;
    let prepared = super::preparation::prepare_for_job(s, summaries, job).await?;
    if let (Some(candidates), Some(id)) = (&s.candidates, job.snapshot().candidate_id) {
        let candidate = candidates.candidate(project, id).await?;
        job.prepare_phase(
            PreparationPhase::SavingValues,
            "字段与关系分析完成，正在保存完整观测值",
        );
        s.checkpoint(project, job, CandidateState::Running).await?;
        candidates
            .freeze_candidate_values(project, id, job.run_id, &prepared.value_archive)
            .await?;
        job.update(|p| p.values_archived = true);
        job.prepare_phase(
            PreparationPhase::SavingInput,
            "完整观测值已保存，正在保存本轮整理输入",
        );
        s.checkpoint(project, job, CandidateState::Running).await?;
        let frozen = FrozenInput {
            version: 2,
            summaries: summaries.to_vec(),
            prepared: prepared.clone(),
            baseline_batches: candidate.batches.iter().map(|b| b.id).collect(),
            legacy: false,
        };
        candidates
            .freeze_candidate_input(
                project,
                id,
                job.run_id,
                serde_json::to_value(frozen).expect("snapshot"),
            )
            .await?;
        job.update(|p| p.resume_available = Some(true));
    }
    job.prepare_phase(
        PreparationPhase::Ready,
        "整理资料已就绪，开始核对接口名称与用途",
    );
    s.checkpoint(project, job, CandidateState::Running).await?;
    Ok(prepared)
}
#[cfg(test)]
pub(super) async fn legacy_input(
    s: &Curation,
    candidate: &Candidate,
) -> Result<FrozenInput, CurationError> {
    let summaries = s
        .endpoints
        .list(candidate.project_id)
        .await
        .map_err(|_| CurationError::Unavailable)?;
    let prepared = super::preparation::prepare_legacy(s, &summaries).await?;
    let progress: Progress = serde_json::from_value(candidate.progress.clone()).unwrap_or_default();
    let mut total = 0;
    let mut missing = 0;
    for facts in prepared
        .facts
        .iter()
        .filter(|f| candidate.target.is_none_or(|id| id == f.summary.id))
    {
        let view = nexofolio_knowledge::CandidateSession::new(
            candidate,
            s.candidates.as_ref().unwrap().clone(),
        );
        let knowledge =
            nexofolio_contracts::knowledge::KnowledgeReader::endpoint(&view, facts.summary.id)
                .await?;
        let inputs = prepared
            .inputs
            .get(&facts.summary.id)
            .cloned()
            .unwrap_or_default();
        total += inputs.len();
        let pending = nexofolio_curate::pending_fields(facts, &knowledge, inputs);
        // Any accepted field becoming stale means we cannot prove the original inputs.
        if pending.iter().any(|i| {
            knowledge
                .fields
                .iter()
                .any(|n| n.path == i.key && n.context.input_hash.is_some())
        }) {
            return Err(CurationError::Failed("旧轮次未保存冻结输入，当前证据与已保存字段不相容；为保护已有结果，请开始新一轮整理".into()));
        }
        missing += pending.len();
    }
    if total != progress.fields_total + progress.fields_reused
        || missing
            != progress
                .fields_total
                .saturating_sub(progress.fields_updated)
    {
        return Err(CurationError::Failed(
            "旧轮次缺少冻结任务清单，现有资料无法核对原缺口；请开始新一轮整理，旧候选保持可审阅"
                .into(),
        ));
    }
    Ok(FrozenInput {
        version: 2,
        summaries,
        prepared,
        baseline_batches: vec![],
        legacy: true,
    })
}
pub(super) async fn execute(
    s: &Curation,
    project: ProjectId,
    summaries: &[EndpointSummary],
    job: &Job,
) -> Result<(), CurationError> {
    super::supervision::supervise(
        job,
        super::run(s, project, summaries, job),
        super::supervision::heartbeat(s, project, job),
        s.idle_budget,
    )
    .await
}
