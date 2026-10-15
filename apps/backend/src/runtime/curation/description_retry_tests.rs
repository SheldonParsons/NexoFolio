//! Endpoint-local retries under a fake provider; never calls a real model or database.
use super::tests::{Memory, fixture};
use super::*;
use async_trait::async_trait;
use nexofolio_curate::CompletionError;
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::time::Instant;

struct Model {
    failures: usize,
    code: &'static str,
    sibling_delay: u64,
    memory: Arc<Memory>,
    project: ProjectId,
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
    sibling_was_saved_on_retry: Mutex<bool>,
}
#[async_trait]
impl Completions for Model {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        let path = if prompt.contains("/api/1") {
            "/api/1"
        } else {
            "/api/2"
        };
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            let times = attempts.entry(path.into()).or_default();
            times.push(Instant::now());
            times.len()
        };
        if path == "/api/1" {
            if attempt > 1 && !self.memory.notes(self.project).await.unwrap().is_empty() {
                *self.sibling_was_saved_on_retry.lock().unwrap() = true;
            }
            if attempt <= self.failures {
                return Err(match self.code {
                    "HTTP 429" => CompletionError::Status {
                        status: 429,
                        retry_after_seconds: Some(30),
                    },
                    "HTTP 401" => CompletionError::Status {
                        status: 401,
                        retry_after_seconds: None,
                    },
                    _ => CompletionError::InvalidReply(self.code.into()),
                });
            }
        } else {
            tokio::time::sleep(Duration::from_secs(self.sibling_delay)).await;
        }
        Ok(serde_json::json!({"name":"合成接口","purpose":"查询合成记录"}).to_string())
    }
}
async fn setup(
    failures: usize,
    code: &'static str,
    sibling_delay: u64,
) -> (
    Curation,
    ProjectId,
    Arc<Memory>,
    Arc<Model>,
    super::preparation::PreparedProject,
    Arc<super::jobs::Job>,
) {
    let (mut s, project, memory, _) = fixture(false);
    let model = Arc::new(Model {
        failures,
        code,
        sibling_delay,
        memory: memory.clone(),
        project,
        attempts: Mutex::default(),
        sibling_was_saved_on_retry: Mutex::new(false),
    });
    s.describe = Arc::new(Describe::new(
        model.clone() as Arc<dyn Completions>,
        "fixture".into(),
    ));
    let summaries = s.endpoints.list(project).await.unwrap();
    let prepared = super::preparation::prepare(&s, &summaries).await.unwrap();
    let (job, _) = s.jobs.start(project, 2);
    (s, project, memory, model, prepared, job)
}
#[tokio::test(start_paused = true)]
async fn transient_429_returns_policy_to_queue_and_preserves_inflight_results() {
    let (s, project, memory, model, prepared, job) = setup(2, "HTTP 429", 3).await;
    let error = super::descriptions::run(
        &s,
        project,
        &job,
        &super::frozen_input::ExecutionInput::legacy(prepared.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        CurationError::Model(CompletionError::Status { status: 429, .. })
    ));
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 1);
    assert_eq!(memory.notes(project).await.unwrap().len(), 1);
    assert_eq!(job.snapshot().retries, 0);
}
#[tokio::test(start_paused = true)]
async fn persistent_429_does_not_multiply_stage_budget_or_cancel_sibling_and_next_run_skips_saved()
{
    let (s, project, memory, model, prepared, job) = setup(usize::MAX, "HTTP 429", 215).await;
    let summaries = s.endpoints.list(project).await.unwrap();
    let result = super::stages::run(
        &s,
        project,
        &summaries,
        &job,
        None,
        &super::frozen_input::ExecutionInput::legacy(prepared.clone()),
        Pass::Describe,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 1);
    assert_eq!(model.attempts.lock().unwrap()["/api/2"].len(), 1);
    assert_eq!(memory.notes(project).await.unwrap().len(), 1);
    assert_eq!(job.snapshot().completed, 1);
    assert!(job.snapshot().active.is_empty());
    let fresh = Arc::new(Model {
        failures: 0,
        code: "",
        sibling_delay: 0,
        memory: memory.clone(),
        project,
        attempts: Mutex::default(),
        sibling_was_saved_on_retry: Mutex::new(false),
    });
    let mut resumed = s.clone();
    resumed.resuming = true;
    resumed.describe = Arc::new(Describe::new(
        fresh.clone() as Arc<dyn Completions>,
        "fixture".into(),
    ));
    super::descriptions::run(
        &resumed,
        project,
        &job,
        &super::frozen_input::ExecutionInput::legacy(prepared.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        fresh
            .attempts
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["/api/1"]
    );
    assert_eq!(memory.notes(project).await.unwrap().len(), 2);
}
#[tokio::test(start_paused = true)]
async fn malformed_reply_retries_locally_and_authentication_rejection_does_not_retry() {
    let (s, project, _, model, prepared, job) = setup(1, "接口说明返回 JSON 格式无效", 0).await;
    super::descriptions::run(
        &s,
        project,
        &job,
        &super::frozen_input::ExecutionInput::legacy(prepared.clone()),
    )
    .await
    .unwrap();
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 2);
    assert_eq!(model.attempts.lock().unwrap()["/api/2"].len(), 1);
    let (s, project, _, model, prepared, job) = setup(usize::MAX, "HTTP 401", 0).await;
    assert!(
        super::descriptions::run(
            &s,
            project,
            &job,
            &super::frozen_input::ExecutionInput::legacy(prepared.clone())
        )
        .await
        .is_err()
    );
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 1);
}
#[tokio::test(start_paused = true)]
async fn cancellation_during_backoff_stops_retries() {
    let (s, project, _, model, prepared, job) = setup(usize::MAX, "HTTP 429", 0).await;
    let cancel = async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        job.cancel();
        std::future::pending().await
    };
    let result = super::supervision::supervise(
        &job,
        super::descriptions::run(
            &s,
            project,
            &job,
            &super::frozen_input::ExecutionInput::legacy(prepared.clone()),
        ),
        cancel,
        Duration::from_secs(900),
    )
    .await;
    assert!(result.is_err());
    tokio::time::advance(Duration::from_secs(300)).await;
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 1);
}

#[tokio::test(start_paused = true)]
async fn restored_invalid_reply_count_does_not_restart_the_local_repair_budget() {
    let (s, project, _, model, prepared, job) = setup(usize::MAX, "invalid JSON", 0).await;
    let endpoint = prepared
        .facts
        .iter()
        .find(|f| f.summary.path_template == "/api/1")
        .unwrap()
        .summary
        .id;
    job.update(|p| {
        p.validation_failures
            .insert(format!("describe/{endpoint}"), 3);
    });
    assert!(
        super::descriptions::run(
            &s,
            project,
            &job,
            &super::frozen_input::ExecutionInput::legacy(prepared.clone())
        )
        .await
        .is_err()
    );
    assert_eq!(model.attempts.lock().unwrap()["/api/1"].len(), 1);
    let restored: Progress =
        serde_json::from_value(serde_json::to_value(job.snapshot()).unwrap()).unwrap();
    assert_eq!(
        restored.validation_failures[&format!("describe/{endpoint}")],
        4
    );
}
