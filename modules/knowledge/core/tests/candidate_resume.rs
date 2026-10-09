#[allow(dead_code)]
#[path = "candidates/support.rs"]
mod support;
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::json;
use support::*;

#[tokio::test]
async fn same_id_claim_freezes_input_fences_workers_and_retains_acknowledged_batches() {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let k = Knowledge::new(InMemoryKnowledge::new(), facts);
    let c = k.create_candidate(project, None, None).await.unwrap();
    let first = uuid::Uuid::new_v4();
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Running,
        json!({"run_id":first,"fields_updated":0}),
    )
    .await
    .unwrap();
    let input = json!({"version":1,"fixture":"frozen"});
    k.freeze_candidate_input(project, c.id, first, input.clone())
        .await
        .unwrap();
    let batch = k
        .open_candidate_batch(project, c.id, Some(first), author())
        .await
        .unwrap();
    k.append_candidate(
        project,
        c.id,
        Some(first),
        batch,
        &[note(endpoint, "已保存")],
    )
    .await
    .unwrap();
    assert!(matches!(
        k.create_candidate(project, None, None).await,
        Err(KnowledgeError::CandidateBusy)
    ));
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Partial,
        json!({"run_id":first,"fields_updated":0}),
    )
    .await
    .unwrap();
    let next = uuid::Uuid::new_v4();
    assert!(matches!(
        k.resume_candidate(project, c.id, next, json!({"changed":true}))
            .await,
        Err(KnowledgeError::CandidateStale)
    ));
    let restored = k
        .resume_candidate(project, c.id, next, input.clone())
        .await
        .unwrap();
    assert_eq!(restored.id, c.id);
    assert_eq!(restored.created_at, c.created_at);
    assert_eq!(restored.view.notes.len(), 1);
    assert_eq!(restored.batches[0].commands.len(), 1);
    assert_eq!(restored.batches[0].state, RoundState::Failed);
    assert!(
        k.append_candidate(project, c.id, Some(first), batch, &[note(endpoint, "迟到")])
            .await
            .is_err()
    );
    assert!(
        k.open_candidate_batch(project, c.id, Some(first), author())
            .await
            .is_err()
    );
    assert!(
        k.checkpoint_candidate(
            project,
            c.id,
            CandidateState::Ready,
            json!({"run_id":first})
        )
        .await
        .is_err()
    );
    assert!(matches!(
        k.resume_candidate(project, c.id, uuid::Uuid::new_v4(), input.clone())
            .await,
        Err(KnowledgeError::CandidateBusy)
    ));
    k.cancel_candidate(project, c.id).await.unwrap();
    assert!(k.heartbeat_candidate(project, c.id, next).await.is_err());
    assert!(
        k.open_candidate_batch(project, c.id, Some(next), author())
            .await
            .is_err()
    );
    let latest = uuid::Uuid::new_v4();
    k.resume_candidate(project, c.id, latest, input.clone())
        .await
        .unwrap();
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Ready,
        json!({"run_id":latest}),
    )
    .await
    .unwrap();
    k.apply_candidate(project, c.id, nexofolio_common::UserId::new())
        .await
        .unwrap();
    assert!(matches!(
        k.resume_candidate(project, c.id, uuid::Uuid::new_v4(), input)
            .await,
        Err(KnowledgeError::CandidateClosed)
    ));
}

#[tokio::test]
async fn manual_baseline_change_rejects_resume_without_overwriting_it() {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let k = Knowledge::new(InMemoryKnowledge::new(), facts);
    let c = k.create_candidate(project, None, None).await.unwrap();
    k.checkpoint_candidate(project, c.id, CandidateState::Partial, json!({}))
        .await
        .unwrap();
    let batch = k
        .begin(
            project,
            Author::Person {
                user: nexofolio_common::UserId::new(),
            },
        )
        .await
        .unwrap();
    k.apply(batch, &[note(endpoint, "人工修订")]).await.unwrap();
    k.finish(batch, RoundState::Done, None).await.unwrap();
    assert!(matches!(
        k.resume_candidate(project, c.id, uuid::Uuid::new_v4(), json!({}))
            .await,
        Err(KnowledgeError::CandidateStale)
    ));
    assert_eq!(
        k.candidate(project, c.id).await.unwrap().state,
        CandidateState::Partial
    );
    assert_eq!(
        k.endpoint(endpoint)
            .await
            .unwrap()
            .note
            .unwrap()
            .name
            .as_deref(),
        Some("人工修订")
    );
}

#[tokio::test]
async fn expired_worker_can_be_reclaimed_but_heartbeat_prevents_double_ownership() {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };
    struct TestClock(Arc<AtomicI64>);
    impl nexofolio_knowledge::Clock for TestClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst), 0).unwrap()
        }
    }
    let facts = facts();
    let project = facts.0.project_id;
    let clock = Arc::new(AtomicI64::new(1000));
    let k = Knowledge::with_clock(InMemoryKnowledge::new(), facts, TestClock(clock.clone()));
    let c = k.create_candidate(project, None, None).await.unwrap();
    let first = uuid::Uuid::new_v4();
    k.checkpoint_candidate(
        project,
        c.id,
        CandidateState::Running,
        json!({"run_id":first}),
    )
    .await
    .unwrap();
    clock.store(1100, Ordering::SeqCst);
    k.heartbeat_candidate(project, c.id, first).await.unwrap();
    clock.store(1210, Ordering::SeqCst);
    assert!(matches!(
        k.resume_candidate(project, c.id, uuid::Uuid::new_v4(), json!({}))
            .await,
        Err(KnowledgeError::CandidateBusy)
    ));
    clock.store(1221, Ordering::SeqCst);
    let next = uuid::Uuid::new_v4();
    let restored = k
        .resume_candidate(project, c.id, next, json!({}))
        .await
        .unwrap();
    assert_eq!(restored.id, c.id);
    assert_eq!(restored.progress["run_id"], json!(next));
    assert!(k.heartbeat_candidate(project, c.id, first).await.is_err());
    assert!(
        k.open_candidate_batch(project, c.id, Some(first), author())
            .await
            .is_err()
    );
    k.heartbeat_candidate(project, c.id, next).await.unwrap();
}
