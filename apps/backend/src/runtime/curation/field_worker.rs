//! One bounded batch, validated replies and durable acknowledgements. No storage decisions.
use super::jobs::{ActiveEndpoint, ActiveState, Issue};
use nexofolio_common::EndpointId;
use nexofolio_contracts::{
    endpoint::EndpointFacts,
    knowledge::{Command, EndpointKnowledge},
};
use nexofolio_curate::{Completions, Describe, FieldBatchOutput, FieldInput, Pass};
use std::{sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};

pub(super) type Pending = (EndpointId, Vec<Vec<String>>, usize);
pub(super) struct Work {
    pub facts: EndpointFacts,
    pub knowledge: EndpointKnowledge,
}
pub(super) struct Task {
    pub id: u64,
    pub work: Arc<Work>,
    pub inputs: Vec<FieldInput>,
    pub batch: usize,
    pub batches: usize,
}
pub(super) enum Event {
    Started(ActiveEndpoint),
    Call {
        task: u64,
        attempt: usize,
    },
    Response {
        task: u64,
        elapsed_ms: u64,
        success: bool,
    },
    Retry {
        task: u64,
        endpoint: EndpointId,
        code: &'static str,
        fields: usize,
        keys: Vec<String>,
    },
    Saved {
        task: u64,
        endpoint: EndpointId,
        commands: Vec<Command>,
        ack: oneshot::Sender<bool>,
    },
    Failed(Issue),
    Fault(nexofolio_curate::CompletionError),
    Finished {
        task: u64,
        endpoint: EndpointId,
        failed: bool,
        ack: oneshot::Sender<bool>,
    },
}

pub(super) async fn execute(
    describe: Arc<Describe<Arc<dyn Completions>>>,
    task: Task,
    events: mpsc::Sender<Event>,
) {
    let endpoint = task.work.facts.summary.id;
    let _ = events
        .send(Event::Started(ActiveEndpoint {
            task_id: task.id,
            endpoint,
            name: task
                .work
                .knowledge
                .note
                .as_ref()
                .and_then(|n| n.name.clone()),
            path: task.work.facts.summary.path_template.clone(),
            fields: task.inputs.len(),
            completed: 0,
            batch: task.batch,
            batches: task.batches,
            attempt: 1,
            started_at: chrono::Utc::now(),
            state: ActiveState::Model,
        }))
        .await;
    let mut knowledge = task.work.knowledge.clone();
    let mut failed = false;
    let started = Instant::now();
    let mut calls = 0;
    let mut pending = std::collections::VecDeque::from([(task.inputs.clone(), 1usize)]);
    while let Some((inputs, attempt)) = pending.pop_front() {
        if events.is_closed() {
            return;
        }
        let budget = calls >= 32 || started.elapsed().as_secs() >= 900 || attempt > 4;
        if budget {
            failed = true;
            let _ = events
                .send(Event::Failed(issue(
                    endpoint,
                    "retry_budget_exhausted",
                    inputs.len(),
                )))
                .await;
            continue;
        }
        calls += 1;
        let remaining = std::time::Duration::from_secs(900).saturating_sub(started.elapsed());
        let reply = tokio::time::timeout(
            remaining,
            call(&describe, &task, &knowledge, &inputs, attempt, &events),
        )
        .await;
        let (missing, code) = match reply {
            Ok(Ok(output)) => {
                let commands = output.commands;
                if !save(&task, commands.clone(), &events).await {
                    return;
                }
                for command in commands {
                    if let Command::PutFieldNote {
                        endpoint,
                        path,
                        text,
                        values,
                        context,
                    } = command
                    {
                        knowledge.fields.retain(|n| n.path != path);
                        knowledge
                            .fields
                            .push(nexofolio_contracts::knowledge::FieldNote {
                                endpoint,
                                path,
                                text,
                                values,
                                context,
                                author: describe.author(),
                                at: chrono::Utc::now(),
                            });
                    }
                }
                (output.missing, "missing_fields")
            }
            Ok(Err(error @ nexofolio_curate::CompletionError::InvalidReply(_))) => {
                (inputs, error.code())
            }
            Ok(Err(error)) => {
                let _ = events.send(Event::Fault(error)).await;
                std::future::pending::<()>().await;
                return;
            }
            Err(_) => (inputs, "retry_budget_exhausted"),
        };
        if missing.is_empty() {
            continue;
        }
        if code == "model_rejected" || code == "retry_budget_exhausted" || attempt >= 4 {
            failed = true;
            let _ = events
                .send(Event::Failed(issue(endpoint, code, missing.len())))
                .await;
            continue;
        }
        retry(&task, code, &missing, &events).await;
        // Missing-only keeps all accepted fields. Repeated malformed/timeout batches fall back to single fields.
        let size = if code == "missing_fields" || code == "model_rate_limited" {
            missing.len()
        } else if attempt >= 2 {
            1
        } else {
            missing.len().div_ceil(2)
        };
        for group in missing.chunks(size.max(1)) {
            pending.push_back((group.to_vec(), attempt + 1));
        }
    }
    let (ack, saved) = oneshot::channel();
    if events
        .send(Event::Finished {
            task: task.id,
            endpoint,
            failed,
            ack,
        })
        .await
        .is_ok()
    {
        let _ = saved.await;
    }
}

async fn call(
    describe: &Describe<Arc<dyn Completions>>,
    task: &Task,
    knowledge: &EndpointKnowledge,
    inputs: &[FieldInput],
    attempt: usize,
    events: &mpsc::Sender<Event>,
) -> Result<FieldBatchOutput, nexofolio_curate::CompletionError> {
    let _ = events
        .send(Event::Call {
            task: task.id,
            attempt,
        })
        .await;
    let started = Instant::now();
    let reply = describe
        .fields_batch_partial(&task.work.facts, knowledge, inputs)
        .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = events
        .send(Event::Response {
            task: task.id,
            elapsed_ms,
            success: reply.is_ok(),
        })
        .await;
    tracing::info!(
        task = task.id,
        batch = task.batch,
        fields = inputs.len(),
        attempt,
        elapsed_ms,
        success = reply.is_ok(),
        "field model turn finished"
    );
    reply
}
async fn retry(
    task: &Task,
    code: &'static str,
    inputs: &[FieldInput],
    events: &mpsc::Sender<Event>,
) {
    let fields = inputs.len();
    tracing::warn!(endpoint = %task.work.facts.summary.id, batch = task.batch, fields, code, "field batch retry");
    let _ = events
        .send(Event::Retry {
            task: task.id,
            endpoint: task.work.facts.summary.id,
            code,
            fields,
            keys: inputs
                .iter()
                .map(|input| format!("{}/{}", task.work.facts.summary.id, input.key))
                .collect(),
        })
        .await;
}
async fn save(task: &Task, commands: Vec<Command>, events: &mpsc::Sender<Event>) -> bool {
    if commands.is_empty() {
        return true;
    }
    let (ack, saved) = oneshot::channel();
    events
        .send(Event::Saved {
            task: task.id,
            endpoint: task.work.facts.summary.id,
            commands,
            ack,
        })
        .await
        .is_ok()
        && saved.await.unwrap_or(false)
}
fn issue(endpoint: EndpointId, code: &str, fields: usize) -> Issue {
    Issue {
        endpoint,
        stage: Pass::Fields,
        code: code.into(),
        fields,
        message: match code {
            "retry_budget_exhausted" => "本批补跑预算已用完（每字段最多 4 次、每批 32 次调用或 15 分钟）；已保存字段保留，收尾将复查缺口",
            "missing_fields" => "模型持续遗漏部分字段，已保存字段保留",
            "model_timeout" => "模型响应超时，已完成的批次已保留",
            "invalid_reply" => "部分字段回复缺失或格式无效，已校验通过的字段已保留",
            "model_rejected" => "模型服务拒绝请求，请检查配置或服务状态",
            "model_rate_limited" => "模型服务限流，已完成的批次已保留",
            _ => "模型暂时不可用，已完成的批次已保留",
        }
        .into(),
        retryable: true,
    }
}
