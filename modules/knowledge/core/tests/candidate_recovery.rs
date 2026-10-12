mod recovery {
    pub mod support;
}
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use recovery::support::*;

#[tokio::test]
async fn restart_keeps_saved_work_and_fences_old_owner() {
    retained_work_and_single_owner(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn manual_stop_is_not_automatically_resumed() {
    manual_stop_wins_after_scan(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn a_live_workers_renewed_heartbeat_prevents_takeover() {
    renewed_heartbeat_wins_after_scan(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn incompatible_resume_keeps_data_and_stops_automatic_retries() {
    missing_input_and_changed_baseline_are_retained_and_blocked(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn scan_uses_keyset_pages_and_excludes_stopped_or_reviewed_work() {
    scan_pages_without_loading_or_resuming_closed_tasks(InMemoryKnowledge::new()).await;
}

#[tokio::test]
async fn takeover_requires_the_existing_120_second_lease_to_expire() {
    use nexofolio_contracts::knowledge::*;
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };
    struct Clock(Arc<AtomicI64>);
    impl nexofolio_knowledge::Clock for Clock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::DateTime::from_timestamp(self.0.load(Ordering::SeqCst), 0).unwrap()
        }
    }
    let facts = facts();
    let p = facts.0.project_id;
    let now = Arc::new(AtomicI64::new(1000));
    let k = nexofolio_knowledge::Knowledge::with_clock(
        InMemoryKnowledge::new(),
        facts,
        Clock(now.clone()),
    );
    let c = k.create_candidate(p, None, None).await.unwrap();
    let owner = uuid::Uuid::new_v4();
    k.checkpoint_candidate(
        p,
        c.id,
        CandidateState::Running,
        serde_json::json!({"run_id":owner}),
    )
    .await
    .unwrap();
    k.freeze_candidate_input(p, c.id, owner, serde_json::json!({"version":2}))
        .await
        .unwrap();
    now.store(1119, Ordering::SeqCst);
    assert!(k.recovery_candidates(None, 20).await.unwrap().is_empty());
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    now.store(1120, Ordering::SeqCst);
    assert_eq!(k.recovery_candidates(None, 20).await.unwrap().len(), 1);
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_some()
    );
}
