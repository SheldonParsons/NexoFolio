//! Persist generic protocol/structure notes through the same ownership and ACK boundary.
use super::{Curation, CurationError, activity::Activity, frozen_input::ExecutionInput, jobs::Job};
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::{Author, CandidateState, Command, RoundState};
use nexofolio_curate::Pass;
pub(super) async fn run(
    s: &Curation,
    project: ProjectId,
    job: &Job,
    prepared: &ExecutionInput,
) -> Result<(), CurationError> {
    let mut commands = Vec::new();
    let target = job.snapshot().target;
    for endpoint in prepared
        .endpoints
        .iter()
        .copied()
        .filter(|e| target.is_none_or(|id| *e == id))
    {
        let loaded = prepared.endpoint(s, endpoint).await?;
        let facts = &loaded.facts;
        let knowledge = s.reader.endpoint(endpoint).await?;
        let inputs = loaded.inputs;
        for input in nexofolio_curate::pending_fields(facts, &knowledge, inputs) {
            if let Some(mut command) =
                nexofolio_curate::standard_field_command(facts, &knowledge, &input)
            {
                if let Command::PutFieldNote { context, .. } = &mut command
                    && let Some(observations) = &mut context.observations
                {
                    observations.archive_id = job.snapshot().candidate_id;
                }
                commands.push(command);
            }
        }
    }
    if commands.is_empty() {
        return Ok(());
    }
    let round = s
        .knowledge
        .begin(
            project,
            Author::Curate {
                pass: "fields".into(),
                model: None,
            },
        )
        .await?;
    for chunk in commands.chunks(32) {
        let started = std::time::Instant::now();
        s.knowledge.apply(round, chunk).await?;
        job.update(|p| {
            p.save_ms += started.elapsed().as_millis() as u64;
            for command in chunk {
                if let Command::PutFieldNote {
                    endpoint,
                    path,
                    context,
                    ..
                } = command
                {
                    let key = format!("{endpoint}/{path}");
                    p.saved_field_keys.insert(key.clone());
                    p.field_outcomes
                        .insert(key, super::field_status::outcome(context));
                }
            }
            p.fields_updated = p.saved_field_keys.len();
            super::field_status::counts(p);
        });
        job.record(
            Activity::new(
                Pass::Fields,
                "saved",
                format!(
                    "已按通用规范与结构补齐 {} 个字段，实际观测值和来源分别保留",
                    chunk.len()
                ),
            )
            .fields(chunk.len()),
        );
        s.checkpoint(project, job, CandidateState::Running).await?;
    }
    s.knowledge
        .finish(
            round,
            RoundState::Done,
            Some(format!("规则补齐 {} 个字段", commands.len())),
        )
        .await?;
    Ok(())
}
