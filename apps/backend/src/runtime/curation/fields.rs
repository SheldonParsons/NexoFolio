//! Field IO, bounded concurrency and incremental writes. Freshness belongs to curate.
use super::{Curation, CurationError, jobs::Job, preparation::PreparedProject};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::RoundState;
use nexofolio_curate::pending_fields;

pub(super) async fn run(
    s: &Curation,
    project: ProjectId,
    job: &Job,
    target: Option<nexofolio_common::EndpointId>,
    prepared: &PreparedProject,
) -> Result<(), CurationError> {
    job.update(|p| {
        p.fields_completed = 0;
        p.fields_failed = 0;
        p.fields_reused = 0;
        p.issues
            .retain(|i| i.stage != nexofolio_curate::Pass::Fields);
    });
    let links = s.reader.links(project).await?;
    let commands = nexofolio_curate::relation_commands(&prepared.links, &links, target);
    if !commands.is_empty() {
        let round = s
            .knowledge
            .begin(
                project,
                nexofolio_contracts::knowledge::Author::Curate {
                    pass: "navigation".into(),
                    model: None,
                },
            )
            .await?;
        let result = s.knowledge.apply(round, &commands).await;
        s.knowledge
            .finish(
                round,
                if result.is_ok() {
                    RoundState::Done
                } else {
                    RoundState::Failed
                },
                Some(format!("更新 {} 条有来源的接口关系", commands.len())),
            )
            .await?;
        result?;
    }
    super::field_status::refresh(s, prepared, job).await?;
    job.update(|p| p.fields_total = p.fields_total.max(p.fields_pending));
    super::standard_pass::run(s, project, job, prepared).await?;
    let mut pending = Vec::new();
    for facts in prepared
        .facts
        .iter()
        .filter(|facts| target.is_none_or(|id| facts.summary.id == id))
        .cloned()
    {
        let knowledge = s.reader.endpoint(facts.summary.id).await?;
        let all_inputs = prepared
            .inputs
            .get(&facts.summary.id)
            .cloned()
            .unwrap_or_default();
        let all_count = all_inputs.len();
        let inputs = pending_fields(&facts, &knowledge, all_inputs);
        job.update(|p| p.fields_reused += all_count - inputs.len());
        if inputs.is_empty() {
            job.update(|p| p.fields_completed += 1);
        } else {
            pending.push((facts, knowledge, inputs));
        }
    }
    job.update(|p| p.fields_reused = p.fields_reused.saturating_sub(p.fields_updated));
    job.record(super::activity::Activity::new(
        nexofolio_curate::Pass::Fields,
        "reuse",
        format!(
            "复用 {} 条未变化的字段说明，{} 个接口需要更新",
            job.snapshot().fields_reused,
            pending.len()
        ),
    ));
    if pending.is_empty() {
        return Ok(());
    }
    pending.sort_by_key(|(_, _, inputs)| inputs.len());
    let round = s.knowledge.begin(project, s.describe.author()).await?;
    tracing::info!(round = %round, endpoints = pending.len(), "field round opened");
    let result = super::field_scheduler::run(s, project, round, pending, job).await;
    super::field_status::refresh(s, prepared, job).await?;
    s.knowledge
        .finish(
            round,
            if result.is_ok() {
                RoundState::Done
            } else {
                RoundState::Failed
            },
            Some(format!(
                "更新 {} 个字段的说明与依据",
                job.snapshot().fields_updated
            )),
        )
        .await?;
    result
}

/// Re-read acknowledged knowledge before declaring coverage complete.
pub(super) async fn coverage(
    s: &Curation,
    prepared: &PreparedProject,
) -> Result<(usize, usize), CurationError> {
    let mut fields = 0;
    let mut endpoints = 0;
    for facts in &prepared.facts {
        let knowledge = s.reader.endpoint(facts.summary.id).await?;
        let remaining = pending_fields(
            facts,
            &knowledge,
            prepared
                .inputs
                .get(&facts.summary.id)
                .cloned()
                .unwrap_or_default(),
        );
        fields += remaining.len();
        endpoints += usize::from(!remaining.is_empty());
    }
    Ok((fields, endpoints))
}
