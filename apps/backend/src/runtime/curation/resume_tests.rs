//! Same-ID continuation against durable candidate ports, without real model IO.
use super::tests::{Endpoints, fixture};
use super::*;
use async_trait::async_trait;
use nexofolio_contracts::endpoint::{FieldFacts, FieldLocation, FieldPath, PathSegment, ValueType};
use nexofolio_curate::CompletionError;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
struct Model {
    fail: AtomicBool,
    paths: Mutex<Vec<Vec<String>>>,
}
#[async_trait]
impl Completions for Model {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        let Some(raw) = prompt.split("字段和可核查的证据：").nth(1) else {
            return Ok(json!({"name":"接口","purpose":"查询合成记录"}).to_string());
        };
        let inputs: Vec<Value> = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        self.paths.lock().unwrap().push(
            inputs
                .iter()
                .map(|v| v["path"].as_str().unwrap().into())
                .collect(),
        );
        if inputs.len() == 1 && inputs[0]["path"] == "f2" && self.fail.load(Ordering::SeqCst) {
            return Err(CompletionError("HTTP 401".into()));
        }
        Ok(json!({"fields":inputs.iter().filter(|i|i["path"]!="f2" || !self.fail.load(Ordering::SeqCst)).map(|i|json!({"key":i["key"],"display_name":"合成字段","description":"合成业务字段"})).collect::<Vec<_>>()}).to_string())
    }
}
async fn settled(s: &Curation, project: ProjectId) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while s.progress(project).status == Status::Running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn explicit_resume_keeps_id_count_history_and_calls_only_one_remaining_field() {
    let (template, project, _, _) = fixture(false);
    let mut facts = template
        .endpoints
        .get(template.endpoints.list(project).await.unwrap()[0].id)
        .await
        .unwrap()
        .unwrap();
    facts.fields = (1..=3)
        .map(|i| FieldFacts {
            location: FieldLocation::RequestBody,
            path: FieldPath(vec![PathSegment::Key(format!("f{i}"))]),
            types: vec![ValueType::String],
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        })
        .collect();
    let endpoint = facts.summary.id;
    let endpoints = Endpoints(vec![facts]);
    let knowledge = Arc::new(Knowledge::new(
        InMemoryKnowledge::new(),
        Endpoints(endpoints.0.clone()),
    ));
    let model = Arc::new(Model {
        fail: AtomicBool::new(true),
        paths: Mutex::default(),
    });
    let completions: Arc<dyn Completions> = model.clone();
    let s = Curation::new(
        knowledge.clone(),
        knowledge.clone(),
        Arc::new(endpoints),
        Arc::new(Describe::new(completions.clone(), "fixture".into())),
        Arc::new(Organize::new(completions, "fixture".into())),
    )
    .with_candidates(knowledge.clone());
    let first = s.start_target(project, Some(endpoint), None).await.unwrap();
    settled(&s, project).await;
    let id = first.candidate_id.unwrap();
    // Wait for the terminal durable checkpoint, not just the in-memory status.
    while knowledge.candidate(project, id).await.unwrap().state == CandidateState::Running {
        tokio::task::yield_now().await;
    }
    assert_eq!(s.progress(project).fields_updated, 2);
    let previous = knowledge.candidate(project, id).await.unwrap();
    assert!(previous.execution_input.is_some());
    model.paths.lock().unwrap().clear();
    model.fail.store(false, Ordering::SeqCst);
    let resumed = s.resume_candidate(project, id).await.unwrap();
    assert_eq!(resumed.candidate_id, Some(id));
    settled(&s, project).await;
    while knowledge.candidate(project, id).await.unwrap().state == CandidateState::Running {
        tokio::task::yield_now().await;
    }
    let progress = s.progress(project);
    assert_eq!(progress.status, Status::Done);
    assert_eq!(progress.fields_updated, 3);
    assert_eq!(progress.fields_total, 3);
    assert_eq!(progress.attempts, 2);
    assert_eq!(*model.paths.lock().unwrap(), vec![vec!["f2".to_string()]]);
    let saved = knowledge.candidate(project, id).await.unwrap();
    assert_eq!(saved.created_at, previous.created_at);
    assert_eq!(saved.view.fields.len(), 3);
    assert!(saved.batches.len() > previous.batches.len());
    assert!(
        knowledge
            .endpoint(endpoint)
            .await
            .unwrap()
            .fields
            .is_empty()
    );
}

struct WaitingModel(AtomicBool);
#[async_trait]
impl Completions for WaitingModel {
    async fn complete(&self, _: &str) -> Result<String, CompletionError> {
        self.0.store(true, Ordering::SeqCst);
        std::future::pending().await
    }
}
#[tokio::test]
async fn cancelling_a_model_wait_closes_the_candidate_and_stops_the_worker() {
    let (template, project, _, _) = fixture(false);
    let facts = template
        .endpoints
        .get(template.endpoints.list(project).await.unwrap()[0].id)
        .await
        .unwrap()
        .unwrap();
    let endpoint = facts.summary.id;
    let k = Arc::new(Knowledge::new(
        InMemoryKnowledge::new(),
        Endpoints(vec![facts.clone()]),
    ));
    let model = Arc::new(WaitingModel(AtomicBool::new(false)));
    let completions: Arc<dyn Completions> = model.clone();
    let s = Curation::new(
        k.clone(),
        k.clone(),
        Arc::new(Endpoints(vec![facts])),
        Arc::new(Describe::new(completions.clone(), "fixture".into())),
        Arc::new(Organize::new(completions, "fixture".into())),
    )
    .with_candidates(k.clone());
    let initial = s.start_target(project, Some(endpoint), None).await.unwrap();
    let id = initial.candidate_id.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !model.0.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    s.cancel(project, id).await.unwrap();
    settled(&s, project).await;
    let saved = k.candidate(project, id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert!(saved.view.notes.is_empty());
    assert_eq!(saved.progress["attempt_history"][0]["outcome"], "cancelled");
    assert!(
        s.saved_progress(project)
            .await
            .unwrap()
            .message
            .unwrap()
            .contains("用户已停止")
    );
}
