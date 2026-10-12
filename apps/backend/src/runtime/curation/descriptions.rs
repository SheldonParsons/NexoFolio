//! Description IO and writes; model decisions are delegated to curate.
use super::{Curation, CurationError, jobs::Job};
use super::{
    activity::Activity,
    jobs::{ActiveEndpoint, ActiveState},
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{
    endpoint::EndpointFacts,
    knowledge::{Command, RoundState},
};
use std::collections::HashMap;
use std::time::Instant;
use tokio::task::JoinSet;
use tracing::Instrument;

pub(super) async fn run(
    s: &Curation,
    project: ProjectId,
    job: &Job,
    prepared: &super::preparation::PreparedProject,
) -> Result<(), CurationError> {
    let notes: HashMap<_, _> = s
        .reader
        .notes(project)
        .await?
        .into_iter()
        .map(|n| (n.endpoint, n))
        .collect();
    job.update(|p| {
        p.completed = 0;
        p.unchanged = 0;
    });
    let mut pending = Vec::new();
    for facts in &prepared.facts {
        let input = nexofolio_curate::endpoint_input_from_fields(
            facts,
            prepared
                .inputs
                .get(&facts.summary.id)
                .cloned()
                .unwrap_or_default(),
            prepared
                .authentication
                .get(&facts.summary.id)
                .cloned()
                .unwrap_or_default(),
        );
        let previous = notes.get(&facts.summary.id).cloned();
        if !(s.resuming
            && previous.as_ref().is_some_and(|n| {
                n.name.is_some() && n.purpose.is_some() && n.context.input_hash.is_some()
            }))
            && nexofolio_curate::description_pending(&input, previous.as_ref())
        {
            pending.push((facts.clone(), input, previous));
        } else {
            job.update(|p| {
                p.completed += 1;
                p.unchanged += 1;
            });
        }
    }
    job.update(|p| p.unchanged = p.completed.saturating_sub(p.generated));
    if pending.is_empty() {
        return Ok(());
    }

    let round = s.knowledge.begin(project, s.describe.author()).await?;
    tracing::info!(round = %round, endpoints = pending.len(), "description round opened");
    let result = describe_pending(s, project, round, pending, job).await;
    job.update(|p| p.unchanged = p.completed.saturating_sub(p.generated));
    let count = job.snapshot().generated;
    let state = if result.is_ok() {
        RoundState::Done
    } else {
        RoundState::Failed
    };
    s.knowledge
        .finish(round, state, Some(format!("补全 {count} 个接口说明")))
        .await?;
    result
}

async fn describe_pending(
    s: &Curation,
    project: ProjectId,
    round: nexofolio_common::RoundId,
    facts: Vec<(
        EndpointFacts,
        nexofolio_curate::EndpointInput,
        Option<nexofolio_contracts::knowledge::EndpointNote>,
    )>,
    job: &Job,
) -> Result<(), CurationError> {
    let mut pending = facts.into_iter();
    let mut requests = JoinSet::new();
    for work in pending.by_ref().take(s.description_concurrency) {
        launch(s, work, 1, 0, round, job, &mut requests);
    }
    let mut failure = None;
    while let Some(result) = requests.join_next().await {
        let (work, attempt, model_ms, result) =
            result.map_err(|_| CurationError::Failed("接口说明生成被中断，请重试。".into()))?;
        let endpoint = work.0.summary.id;
        job.update(|p| {
            p.model_ms += model_ms;
            p.active.retain(|a| a.endpoint != endpoint);
        });
        let result = result.and_then(|command| match command {
            Some(
                command @ Command::PutEndpointNote {
                    name: Some(_),
                    purpose: Some(_),
                    ..
                },
            ) => Ok(command),
            _ => Err(nexofolio_curate::CompletionError(
                "接口说明缺少完整名称和用途，请重试".into(),
            )),
        });
        let command = match result {
            Ok(command) => command,
            Err(error) => {
                let code = super::field_worker::error_code(&error.0);
                tracing::warn!(endpoint = %endpoint, attempt, code, "description request failed");
                if code == "model_rejected" {
                    return Err(error.into());
                }
                if attempt < 4 && failure.is_none() {
                    let delay = match code {
                        "model_rate_limited" => 30 * 2u64.pow((attempt - 1) as u32),
                        "model_unavailable" => 2u64.pow(attempt as u32),
                        _ => 1,
                    };
                    job.update(|p| p.retries += 1);
                    let reason = match code {
                        "model_rate_limited" => "模型服务限流，等待额度恢复",
                        "model_timeout" => "模型请求超时",
                        "invalid_reply" | "missing_fields" => "接口说明回复未通过完整性校验",
                        _ => "模型服务暂时不可用",
                    };
                    job.record(Activity::new(nexofolio_curate::Pass::Describe, "retry",
                        format!("{reason}；至少等待 {delay} 秒后仅重试该接口（第 {}/4 次），其他已完成结果保留", attempt+1))
                        .endpoint(endpoint));
                    s.checkpoint(
                        project,
                        job,
                        nexofolio_contracts::knowledge::CandidateState::Running,
                    )
                    .await?;
                    launch(s, work, attempt + 1, delay, round, job, &mut requests);
                } else {
                    job.record(Activity::new(nexofolio_curate::Pass::Describe,"gap",
                        "该接口重试后仍未完成，停止安排新接口，正在保存其他在途结果；之后可续跑本轮")
                        .endpoint(endpoint));
                    failure.get_or_insert_with(|| {
                        CurationError::Failed(format!(
                            "接口说明重试后仍未完成，已保存结果保留，可续跑本轮；{error}"
                        ))
                    });
                }
                continue;
            }
        };
        let saved_name = match &command {
            Command::PutEndpointNote {
                name: Some(name), ..
            } => name.clone(),
            _ => String::new(),
        };
        let saving = Instant::now();
        s.knowledge.apply(round, &[command]).await?;
        let save_ms = saving.elapsed().as_millis() as u64;
        job.record(
            Activity::new(
                nexofolio_curate::Pass::Describe,
                "saved",
                format!("已保存「{saved_name}」的名称、用途和依据"),
            )
            .endpoint(endpoint)
            .elapsed(save_ms),
        );
        job.update(|p| {
            p.completed += 1;
            p.generated += 1;
            p.save_ms += save_ms;
        });
        s.checkpoint(
            project,
            job,
            nexofolio_contracts::knowledge::CandidateState::Running,
        )
        .await?;
        if failure.is_none()
            && let Some(work) = pending.next()
        {
            launch(s, work, 1, 0, round, job, &mut requests);
        }
    }
    failure.map_or(Ok(()), Err)
}

type DescriptionWork = (
    EndpointFacts,
    nexofolio_curate::EndpointInput,
    Option<nexofolio_contracts::knowledge::EndpointNote>,
);
type DescriptionResult = (
    DescriptionWork,
    usize,
    u64,
    Result<Option<Command>, nexofolio_curate::CompletionError>,
);
fn launch(
    s: &Curation,
    work: DescriptionWork,
    attempt: usize,
    delay: u64,
    round: nexofolio_common::RoundId,
    job: &Job,
    requests: &mut JoinSet<DescriptionResult>,
) {
    let (facts, _, previous) = &work;
    let endpoint = facts.summary.id;
    let name = previous
        .as_ref()
        .and_then(|n| n.name.clone())
        .unwrap_or_else(|| facts.summary.path_template.clone());
    if attempt == 1 {
        job.record(
            Activity::new(
                nexofolio_curate::Pass::Describe,
                "started",
                format!("正在核对「{name}」的名称与用途"),
            )
            .endpoint(endpoint),
        );
    }
    job.update(|p| {
        p.model_calls += 1;
        p.active.push(ActiveEndpoint {
            task_id: 0,
            endpoint,
            name: Some(name),
            path: facts.summary.path_template.clone(),
            fields: 0,
            completed: 0,
            batch: 1,
            batches: 1,
            attempt,
            started_at: chrono::Utc::now(),
            state: ActiveState::Model,
        });
    });
    let describe = s.describe.clone();
    requests.spawn(
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            let (facts, input, previous) = &work;
            let started = Instant::now();
            let result = describe
                .endpoint_with_input(facts, input, previous.as_ref())
                .await;
            (work, attempt, started.elapsed().as_millis() as u64, result)
        }
        .instrument(tracing::info_span!("curation_endpoint", endpoint = %endpoint, round = %round)),
    );
}
