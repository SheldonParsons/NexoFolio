//! Fair batch queue with a bounded worker pool. Persistence stays in one receiving task.
use super::{
    Curation, CurationError,
    activity::Activity,
    field_worker::{self, Event, Pending, Task, Work},
    jobs::{ActiveState, Job},
};
use nexofolio_common::{EndpointId, ProjectId, RoundId};
use nexofolio_curate::Pass;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};
use tokio::{sync::mpsc, task::JoinSet};
use tracing::Instrument;

struct QueuedBatch {
    id: u64,
    endpoint: EndpointId,
    keys: Vec<String>,
    batch: usize,
    batches: usize,
}

pub(super) async fn run(
    s: &Curation,
    project: ProjectId,
    round: RoundId,
    pending: Vec<Pending>,
    job: &Job,
    prepared: &super::frozen_input::ExecutionInput,
) -> Result<(), CurationError> {
    let total = pending.iter().map(|(_, _, count)| count).sum();
    let mut groups = Vec::new();
    let mut remaining = HashMap::new();
    for (endpoint, chunks, _) in pending {
        remaining.insert(endpoint, (chunks.len(), false));
        let count = chunks.len();
        groups.push((endpoint, VecDeque::from(chunks), count));
    }
    let mut queue = VecDeque::new();
    loop {
        let before = queue.len();
        for (work, chunks, count) in &mut groups {
            if let Some(inputs) = chunks.pop_front() {
                queue.push_back(QueuedBatch {
                    id: job.snapshot().total_batches as u64 + queue.len() as u64 + 1,
                    endpoint: *work,
                    keys: inputs,
                    batch: *count - chunks.len(),
                    batches: *count,
                });
            }
        }
        if before == queue.len() {
            break;
        }
    }
    job.update(|p| {
        p.fields_total = p.fields_total.max(total);
        p.total_batches = p.completed_batches + queue.len();
        p.queued_batches = queue.len();
    });
    job.record(
        Activity::new(
            Pass::Fields,
            "queued",
            format!(
                "{} 个字段已排为 {} 批，按资料量分批处理",
                total,
                queue.len()
            ),
        )
        .fields(total),
    );
    let (events, mut updates) = mpsc::channel(16);
    let mut workers = JoinSet::new();
    // Provider admission/backoff belongs to the shared adapter budget only.
    // Worker elapsed time includes queueing and must never throttle another layer.
    fill(s, round, &mut queue, &mut workers, &events, job, prepared).await?;
    while !workers.is_empty() {
        let result = tokio::select! {
            Some(event) = updates.recv() => {
                handle(s, project, round, event, &mut remaining, job).await
            },
            result = workers.join_next() => match result {
                Some(Ok(())) => { fill(s, round, &mut queue, &mut workers, &events, job, prepared).await?; Ok(()) }
                _ => Err(CurationError::Failed("字段批次被中断，已保存内容保留，可继续整理".into())),
            }
        };
        if let Err(error) = result {
            workers.abort_all();
            while workers.join_next().await.is_some() {}
            job.update(|p| p.active.clear());
            return Err(error);
        }
    }
    if job.snapshot().fields_failed > 0 {
        Err(CurationError::Failed(format!(
            "{} 个接口有字段未完成，已完成批次保留；可补跑失败部分",
            job.snapshot().fields_failed
        )))
    } else {
        Ok(())
    }
}

async fn fill(
    s: &Curation,
    round: RoundId,
    queue: &mut VecDeque<QueuedBatch>,
    workers: &mut JoinSet<()>,
    events: &mpsc::Sender<Event>,
    job: &Job,
    prepared: &super::frozen_input::ExecutionInput,
) -> Result<(), CurationError> {
    while workers.len() < s.field_concurrency {
        let Some(task) = queue.pop_front() else {
            break;
        };
        let endpoint = task.endpoint;
        let loaded = prepared.batch(s, endpoint, &task.keys).await?;
        let inputs = loaded
            .inputs
            .into_iter()
            .filter(|i| task.keys.contains(&i.key))
            .collect::<Vec<_>>();
        if inputs.len() != task.keys.len() {
            return Err(nexofolio_contracts::knowledge::frozen_invalid().into());
        }
        let knowledge = s.reader.endpoint(endpoint).await?;
        let task = Task {
            id: task.id,
            work: Arc::new(Work {
                facts: loaded.facts,
                knowledge,
            }),
            inputs,
            batch: task.batch,
            batches: task.batches,
        };
        workers.spawn(
            field_worker::execute(s.describe.clone(), task, events.clone()).instrument(
                tracing::info_span!("curation_endpoint", endpoint = %endpoint, round = %round),
            ),
        );
    }
    job.update(|p| p.queued_batches = queue.len());
    Ok(())
}

async fn handle(
    s: &Curation,
    project: ProjectId,
    round: RoundId,
    event: Event,
    remaining: &mut HashMap<EndpointId, (usize, bool)>,
    job: &Job,
) -> Result<(), CurationError> {
    let mut acknowledgement = None;
    match event {
        Event::Fault(error) => return Err(error.into()),
        Event::Started(active) => {
            let name = active.name.as_deref().unwrap_or(&active.path);
            job.record(
                Activity::new(
                    Pass::Fields,
                    "started",
                    format!(
                        "开始处理「{name}」第 {}/{} 批",
                        active.batch, active.batches
                    ),
                )
                .endpoint(active.endpoint)
                .name(active.name.clone())
                .fields(active.fields)
                .batch(active.batch),
            );
            job.update(|p| p.active.push(active));
        }
        Event::Call { task, attempt } => job.update(|p| {
            p.model_calls += 1;
            if let Some(active) = p.active.iter_mut().find(|a| a.task_id == task) {
                active.attempt = attempt;
                active.started_at = chrono::Utc::now();
                active.state = ActiveState::Model;
            }
        }),
        Event::Response {
            task,
            elapsed_ms,
            success,
        } => job.update(|p| {
            p.model_ms += elapsed_ms;
            if success && let Some(active) = p.active.iter_mut().find(|a| a.task_id == task) {
                active.state = ActiveState::Saving;
                active.started_at = chrono::Utc::now();
            }
        }),
        Event::Retry {
            task,
            endpoint,
            code,
            fields,
            keys,
        } => {
            job.update(|p| {
                p.retries += 1;
                for key in &keys {
                    *p.validation_failures.entry(key.clone()).or_default() += 1;
                }
                if let Some(active) = p.active.iter_mut().find(|a| a.task_id == task) {
                    active.attempt = 2;
                }
            });
            if keys.iter().any(|key| {
                job.snapshot()
                    .validation_failures
                    .get(key)
                    .copied()
                    .unwrap_or(0)
                    >= 4
            }) {
                return Err(nexofolio_curate::CompletionError::InvalidReply(
                    "同一字段反复返回无效或不完整结果，需要处理".into(),
                )
                .into());
            }
            let reason = match code {
                "model_timeout" => "模型超时，拆小重试",
                "model_rate_limited" => "模型限流，等待后重试",
                _ => "只重试未通过校验的字段",
            };
            job.record(
                Activity::new(Pass::Fields, "retry", reason)
                    .endpoint(endpoint)
                    .fields(fields),
            );
        }
        Event::Saved {
            task,
            endpoint,
            mut commands,
            ack,
        } => {
            let started = Instant::now();
            for command in &mut commands {
                if let nexofolio_contracts::knowledge::Command::PutFieldNote { context, .. } =
                    command
                    && let Some(values) = &mut context.observations
                {
                    values.archive_id = job.snapshot().candidate_id;
                }
            }
            s.knowledge.apply(round, &commands).await?;

            let elapsed = started.elapsed().as_millis() as u64;
            let name = job
                .snapshot()
                .active
                .iter()
                .find(|a| a.task_id == task)
                .and_then(|a| a.name.clone());
            job.update(|p| {
                let mut newly_addressed = 0;
                for command in &commands {
                    if let nexofolio_contracts::knowledge::Command::PutFieldNote {
                        endpoint,
                        path,
                        context,
                        ..
                    } = command
                    {
                        let key = format!("{endpoint}/{path}");
                        p.saved_field_keys.insert(key.clone());
                        let state = super::field_status::outcome(context);
                        if state != 0 {
                            p.validation_failures.remove(&key);
                        }
                        if p.field_outcomes.get(&key) == Some(&0) && state != 0 {
                            newly_addressed += 1;
                        }
                        p.field_outcomes.insert(key, state);
                    }
                }
                p.fields_updated = p.saved_field_keys.len();
                super::field_status::counts(p);
                p.save_ms += elapsed;
                if let Some(active) = p.active.iter_mut().find(|a| a.task_id == task) {
                    active.completed += newly_addressed;
                }
            });
            job.record(
                Activity::new(
                    Pass::Fields,
                    "saved",
                    format!("已保存 {} 个字段的当前结果，缺项继续补跑", commands.len()),
                )
                .endpoint(endpoint)
                .name(name)
                .fields(commands.len())
                .elapsed(elapsed),
            );
            acknowledgement = Some(ack);
        }
        Event::Failed(issue) => {
            job.record(
                Activity::new(Pass::Fields, "gap", issue.message.clone())
                    .endpoint(issue.endpoint)
                    .fields(issue.fields),
            );
            let permanent = !issue.retryable;
            job.update(|p| p.issues.push(issue));
            if permanent {
                s.checkpoint(
                    project,
                    job,
                    nexofolio_contracts::knowledge::CandidateState::Running,
                )
                .await?;
                return Err(CurationError::Failed(
                    "模型反复返回无效或不完整结果，需要处理；已保存成果保留".into(),
                ));
            }
        }
        Event::Finished {
            task,
            endpoint,
            failed,
            ack,
        } => {
            if let Some((count, any_failed)) = remaining.get_mut(&endpoint) {
                *count -= 1;
                *any_failed |= failed;
                job.update(|p| {
                    if *count == 0 {
                        p.fields_completed += 1;
                        if *any_failed {
                            p.fields_failed += 1;
                        }
                    }
                    p.completed_batches += 1;
                    p.active.retain(|a| a.task_id != task);
                });
            }
            acknowledgement = Some(ack);
        }
    }
    s.checkpoint(
        project,
        job,
        nexofolio_contracts::knowledge::CandidateState::Running,
    )
    .await?;
    if let Some(ack) = acknowledgement {
        let _ = ack.send(true);
    }
    Ok(())
}
