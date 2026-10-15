//! Execution snapshots, same-ID resume and lease-aware cancellation. No semantic rules or SQL.
use super::{Curation, CurationError, Progress, jobs::Job, preparation::PreparedProject};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::{
    endpoint::EndpointSummary,
    knowledge::{Candidate, CandidateState, Command},
};
use std::collections::HashSet;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FrozenInput {
    pub version: u32,
    #[serde(default)]
    pub lifecycle_version: u64,
    pub summaries: Vec<EndpointSummary>,
    #[serde(default)]
    pub prepared: PreparedProject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<nexofolio_contracts::knowledge::FrozenManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    pub baseline_batches: Vec<RoundId>,
    pub legacy: bool,
}
impl Curation {
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
pub(super) fn reconcile(candidate: &Candidate, frozen: &FrozenInput, p: &mut Progress) {
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
) -> Result<super::frozen_input::ExecutionInput, CurationError> {
    use super::{frozen_input::ExecutionInput, preparation_progress::PreparationPhase};
    if let Some(input) = &s.frozen {
        return if let Some(manifest) = &input.manifest {
            ExecutionInput::chunked(
                s,
                manifest.clone(),
                input.summaries.iter().map(|e| e.id).collect(),
            )
            .await
        } else {
            Ok(ExecutionInput::legacy(input.prepared.clone()))
        };
    }
    job.prepare_start(summaries.len());
    s.checkpoint(project, job, CandidateState::Running).await?;
    let prepared =
        if let (Some(candidates), Some(id)) = (&s.candidates, job.snapshot().candidate_id) {
            let frozen = super::chunk_preparation::prepare(s, project, summaries, job).await?;
            job.prepare_phase(
                PreparationPhase::SavingInput,
                "分块资料已保存，正在核验并发布完整清单",
            );
            s.checkpoint(project, job, CandidateState::Running).await?;
            candidates
                .freeze_candidate_input(
                    project,
                    id,
                    job.run_id,
                    serde_json::to_value(&frozen).expect("manifest"),
                )
                .await?;
            job.update(|p| {
                p.resume_available = Some(true);
                p.values_archived = true;
            });
            ExecutionInput::chunked(
                s,
                frozen.manifest.expect("new format"),
                summaries.iter().map(|e| e.id).collect(),
            )
            .await?
        } else {
            ExecutionInput::legacy(super::preparation::prepare_for_job(s, summaries, job).await?)
        };
    job.update(|p| {
        if let Some(preparation) = &mut p.preparation {
            preparation.relations = prepared.links.len();
        }
    });
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
        manifest: None,
        manifest_hash: None,
        lifecycle_version: candidate.base.lifecycle.version,
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
