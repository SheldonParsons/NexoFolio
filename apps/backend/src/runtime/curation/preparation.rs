//! CPU analysis stays outside Tokio's request workers; each project is analyzed once.
use super::{
    Curation, CurationError, jobs::Job, preparation_progress::PreparationPhase,
    snapshot::evidence_snapshot,
};
use nexofolio_common::EndpointId;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary},
    knowledge::AuthenticationUse,
};
use nexofolio_curate::{FieldInput, LinkProposal, ProjectEvidence};
use std::{collections::HashMap, time::Instant};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedProject {
    pub facts: Vec<EndpointFacts>,
    pub inputs: HashMap<EndpointId, Vec<FieldInput>>,
    pub links: Vec<LinkProposal>,
    pub authentication: HashMap<EndpointId, Vec<AuthenticationUse>>,
}
#[cfg(test)]
pub(super) async fn prepare(
    s: &Curation,
    summaries: &[EndpointSummary],
) -> Result<PreparedProject, CurationError> {
    prepare_mode(s, summaries, false, None).await
}
pub(super) async fn prepare_for_job(
    s: &Curation,
    summaries: &[EndpointSummary],
    job: &Job,
) -> Result<PreparedProject, CurationError> {
    prepare_mode(s, summaries, false, Some(job)).await
}
#[cfg(test)]
pub(super) async fn prepare_legacy(
    s: &Curation,
    summaries: &[EndpointSummary],
) -> Result<PreparedProject, CurationError> {
    prepare_mode(s, summaries, true, None).await
}
async fn prepare_mode(
    s: &Curation,
    summaries: &[EndpointSummary],
    legacy: bool,
    job: Option<&Job>,
) -> Result<PreparedProject, CurationError> {
    let (mut facts, mut signals, value_archive, authentication) =
        evidence_snapshot(s, summaries, job).await?;
    if let Some(job) = job {
        job.prepare_phase(
            PreparationPhase::Analyzing,
            "资料读取完成，正在分析字段取值与接口关系",
        );
    }
    if legacy {
        for facts in &mut facts {
            facts.fields.retain(|f| {
                !f.path.0.is_empty()
                    && !matches!(
                        f.location,
                        nexofolio_contracts::endpoint::FieldLocation::RequestHeader
                            | nexofolio_contracts::endpoint::FieldLocation::ResponseHeader { .. }
                    )
            });
        }
        signals.retain(|s| {
            !matches!(
                s.location,
                nexofolio_contracts::endpoint::FieldLocation::RequestHeader
                    | nexofolio_contracts::endpoint::FieldLocation::ResponseHeader { .. }
            ) && ![
                "password",
                "token",
                "secret",
                "authorization",
                "cookie",
                "phone",
                "mobile",
                "email",
                "identitycard",
            ]
            .iter()
            .any(|name| s.path.to_ascii_lowercase().contains(name))
        });
    }
    let span = tracing::Span::current();
    let prepared = offload_cpu(move || {
        span.in_scope(|| {
            let started = Instant::now();
            let (inputs, links) = {
                let evidence = ProjectEvidence::new(&facts, &signals);
                let inputs = facts
                    .iter()
                    .map(|f| {
                        let mut inputs = evidence.fields(f);
                        if !legacy {
                            for input in &mut inputs {
                                if let Some(values) =
                                    value_archive
                                        .fields
                                        .get(&nexofolio_observe::value_field_key(
                                            f.summary.id,
                                            &input.key,
                                        ))
                                {
                                    input.observations =
                                        Some(nexofolio_observe::value_summary(values));
                                    input.value_examples = nexofolio_curate::model_value_examples(
                                        &value_archive,
                                        values,
                                    );
                                }
                            }
                        }
                        (f.summary.id, inputs)
                    })
                    .collect();
                (inputs, evidence.links().to_vec())
            };
            tracing::info!(
                endpoints = facts.len(),
                signals = signals.len(),
                relations = links.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "curation evidence prepared"
            );
            PreparedProject {
                facts,
                inputs,
                links,
                authentication,
            }
        })
    })
    .await?;
    if let Some(job) = job {
        job.update(|p| {
            if let Some(preparation) = &mut p.preparation {
                preparation.relations = prepared.links.len();
            }
        });
    }
    Ok(prepared)
}

async fn offload_cpu<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CurationError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CurationError::Failed("资料分析被中断，原有内容保留，请重试。".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex};
    #[tokio::test(flavor = "current_thread")]
    async fn request_timer_and_progress_can_run_while_cpu_analysis_is_waiting() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_gate = gate.clone();
        let (started, receiver) = tokio::sync::oneshot::channel();
        let work = offload_cpu(move || {
            let _ = started.send(());
            let (lock, notify) = &*worker_gate;
            let (released, _) = notify
                .wait_timeout_while(
                    lock.lock().unwrap(),
                    std::time::Duration::from_secs(1),
                    |v| !*v,
                )
                .unwrap();
            *released
        });
        let request = async {
            receiver.await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let (lock, notify) = &*gate;
            *lock.lock().unwrap() = true;
            notify.notify_one();
        };
        let (released, ()) = tokio::join!(work, request);
        assert!(
            released.unwrap(),
            "request/progress timer must run before CPU work's fallback timeout"
        );
    }
}
