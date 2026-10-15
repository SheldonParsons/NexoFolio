//! Restart orchestration uses persisted results and the existing completion rules.
use super::resume_tests::Model;
use super::tests::{Endpoints, fixture};
use super::*;
use nexofolio_common::UserId;
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::{KnowledgeStore, testing::InMemoryKnowledge};
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

async fn setup() -> (
    Curation,
    Arc<Knowledge<InMemoryKnowledge, Endpoints>>,
    InMemoryKnowledge,
    Arc<Model>,
    ProjectId,
    EndpointId,
) {
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
    let store = InMemoryKnowledge::new();
    let knowledge = Arc::new(Knowledge::new(
        store.clone(),
        Endpoints(vec![facts.clone()]),
    ));
    let model = Arc::new(Model {
        fail: AtomicBool::new(true),
        paths: Mutex::default(),
    });
    let completions: Arc<dyn Completions> = model.clone();
    let curation = Curation::new(
        knowledge.clone(),
        knowledge.clone(),
        Arc::new(Endpoints(vec![facts])),
        Arc::new(Describe::new(completions.clone(), "fixture".into())),
        Arc::new(Organize::new(completions, "fixture".into())),
    )
    .with_candidates(knowledge.clone())
    .with_actor(CandidateActor::User {
        id: UserId::new(),
        display_name: Some("发起人".into()),
    });
    (curation, knowledge, store, model, project, endpoint)
}
async fn terminal(
    k: &Knowledge<InMemoryKnowledge, Endpoints>,
    p: ProjectId,
    id: uuid::Uuid,
) -> Candidate {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let c = k.candidate(p, id).await.unwrap();
            if c.state != CandidateState::Running {
                return c;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn new_executor_automatically_finishes_only_the_unsaved_field() {
    let (mut curation, k, store, model, p, endpoint) = setup().await;
    let first = curation
        .start_target(p, Some(endpoint), None)
        .await
        .unwrap();
    let id = first.candidate_id.unwrap();
    let mut saved = terminal(&k, p, id).await;
    assert_eq!(saved.view.fields.len(), 2, "{:?}", saved.progress);
    let original_input = saved.execution_input.clone();
    let original_actor = saved.operations[0].actor.clone();
    // Persist the last running state a terminated process would leave behind.
    saved.state = CandidateState::Running;
    saved.updated_at -= chrono::TimeDelta::seconds(121);
    saved.progress["status"] = json!("running");
    if let Some(last) = saved.progress["attempt_history"]
        .as_array_mut()
        .and_then(|v| v.last_mut())
    {
        last["ended_at"] = serde_json::Value::Null;
    }
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&saved).await.unwrap();
    tx.commit().await.unwrap();
    curation.jobs = Arc::new(jobs::Jobs::default());
    model.fail.store(false, Ordering::SeqCst);
    model.paths.lock().unwrap().clear();
    curation.recover_expired().await.unwrap();
    let complete = terminal(&k, p, id).await;
    assert_eq!(complete.id, id);
    assert_eq!(complete.state, CandidateState::Ready);
    assert_eq!(complete.created_at, saved.created_at);
    assert_eq!(complete.execution_input, original_input);
    assert_eq!(complete.view.fields.len(), 3);
    assert_eq!(complete.progress["fields_updated"], 3);
    assert_eq!(complete.progress["attempts"], 2);
    assert_eq!(*model.paths.lock().unwrap(), vec![vec!["f2".to_owned()]]);
    assert_eq!(complete.operations[0].actor, original_actor);
    assert!(matches!(
        complete.operations.last().unwrap().actor,
        CandidateActor::System { .. }
    ));
    assert!(
        complete.progress["activity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["message"]
                .as_str()
                .is_some_and(|s| s.contains("系统自动继续")))
    );
    assert!(k.endpoint(endpoint).await.unwrap().fields.is_empty());
    curation.recover_expired().await.unwrap();
    assert_eq!(model.paths.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn incompatible_input_stops_without_model_calls_and_remains_visible() {
    let (curation, k, store, model, p, endpoint) = setup().await;
    let mut c = k.create_candidate(p, Some(endpoint), None).await.unwrap();
    c.execution_input = Some(json!({"version":99,"fixture":"incompatible"}));
    c.updated_at -= chrono::TimeDelta::seconds(121);
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    curation.recover_expired().await.unwrap();
    let saved = k.candidate(p, c.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert_eq!(saved.progress["recovery_blocked"], true);
    assert_eq!(saved.progress["resume_available"], false);
    assert!(model.paths.lock().unwrap().is_empty());
    let shown = curation.saved_progress(p).await.unwrap();
    assert_eq!(shown.resume_available, Some(false));
    assert!(shown.message.unwrap().contains("不兼容"));
    curation.recover_expired().await.unwrap();
    assert_eq!(
        k.candidate(p, c.id).await.unwrap().operations.len(),
        saved.operations.len()
    );
}

#[tokio::test]
async fn cancelled_service_does_not_start_a_recovery_scan() {
    let (curation, k, store, _, p, endpoint) = setup().await;
    let mut c = k.create_candidate(p, Some(endpoint), None).await.unwrap();
    c.updated_at -= chrono::TimeDelta::seconds(121);
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    curation.recover_after_restart(stop).await;
    assert_eq!(
        k.candidate(p, c.id).await.unwrap().state,
        CandidateState::Running
    );
}

#[tokio::test(start_paused = true)]
async fn startup_waits_for_lease_expiry_then_recovers_on_a_later_tick() {
    use std::sync::atomic::AtomicI64;
    struct Clock(Arc<AtomicI64>);
    impl nexofolio_knowledge::Clock for Clock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::Utc::now() + chrono::TimeDelta::seconds(self.0.load(Ordering::SeqCst))
        }
    }
    let (curation, k, store, model, p, endpoint) = setup().await;
    let first = curation
        .start_target(p, Some(endpoint), None)
        .await
        .unwrap();
    let id = first.candidate_id.unwrap();
    let mut c = terminal(&k, p, id).await;
    c.state = CandidateState::Running;
    c.updated_at = chrono::Utc::now();
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    let offset = Arc::new(AtomicI64::new(0));
    let facts = curation.endpoints.get(endpoint).await.unwrap().unwrap();
    let restored = Arc::new(Knowledge::with_clock(
        store,
        Endpoints(vec![facts]),
        Clock(offset.clone()),
    ));
    let mut next = curation.clone();
    next.jobs = Arc::new(jobs::Jobs::default());
    next.reader = restored.clone();
    next.knowledge = restored.clone();
    next.candidates = Some(restored);
    model.fail.store(false, Ordering::SeqCst);
    model.paths.lock().unwrap().clear();
    let stop = CancellationToken::new();
    let worker = tokio::spawn(next.recover_after_restart(stop.clone()));
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    assert!(model.paths.lock().unwrap().is_empty());
    assert_eq!(
        k.candidate(p, id).await.unwrap().operations.len(),
        c.operations.len()
    );
    offset.store(121, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    let done = terminal(&k, p, id).await;
    assert_eq!(done.state, CandidateState::Ready);
    assert_eq!(*model.paths.lock().unwrap(), vec![vec!["f2".to_owned()]]);
    stop.cancel();
    worker.await.unwrap();
}
