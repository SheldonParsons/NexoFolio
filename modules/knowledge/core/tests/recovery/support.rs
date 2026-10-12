//! Shared with PostgreSQL: tests use persisted proposals, never a real model.
#[path = "../candidates/support.rs"]
#[allow(dead_code)]
mod candidates;
pub use candidates::facts;
use candidates::*;
use nexofolio_common::{ProjectId, UserId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::json;

async fn scanned(k: &dyn KnowledgeCandidates) -> Vec<CandidateRecovery> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = k.recovery_candidates(after, 50).await.unwrap();
        let done = page.len() < 50;
        after = page.last().map(|c| c.id);
        all.extend(page);
        if done {
            return all;
        }
    }
}

pub async fn seed<S: KnowledgeStore + Clone>(
    store: &S,
) -> (
    Knowledge<S, Facts>,
    ProjectId,
    Candidate,
    uuid::Uuid,
    UserId,
) {
    let facts = facts();
    let project = facts.0.project_id;
    let knowledge = Knowledge::new(store.clone(), facts);
    let user = UserId::new();
    let candidate = knowledge
        .create_candidate_as(
            project,
            None,
            None,
            CandidateActor::User {
                id: user,
                display_name: Some("原发起人".into()),
            },
        )
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4();
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Running,
            json!({"run_id":owner,"fields_updated":7}),
        )
        .await
        .unwrap();
    knowledge
        .freeze_candidate_input(
            project,
            candidate.id,
            owner,
            json!({"version":2,"fixture":"immutable"}),
        )
        .await
        .unwrap();
    let mut candidate = knowledge.candidate(project, candidate.id).await.unwrap();
    candidate.updated_at -= chrono::TimeDelta::seconds(121);
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&candidate).await.unwrap();
    tx.commit().await.unwrap();
    (knowledge, project, candidate, owner, user)
}

pub async fn retained_work_and_single_owner<S: KnowledgeStore + Clone>(store: S) {
    let (k, project, c, old_owner, user) = seed(&store).await;
    // Save accepted commands in a still-open batch, as a killed worker may leave.
    let batch = k
        .open_candidate_batch(project, c.id, Some(old_owner), author())
        .await
        .unwrap();
    let command = note(c.base.endpoints[0], "已确认保存");
    k.append_candidate(project, c.id, Some(old_owner), batch, &[command])
        .await
        .unwrap();
    let mut before = k.candidate(project, c.id).await.unwrap();
    before.updated_at -= chrono::TimeDelta::seconds(121);
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&before).await.unwrap();
    tx.commit().await.unwrap();
    let owner = uuid::Uuid::new_v4();
    let (a, b) = tokio::join!(
        k.recover_candidate(project, c.id, owner),
        k.recover_candidate(project, c.id, uuid::Uuid::new_v4())
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(usize::from(a.is_some()) + usize::from(b.is_some()), 1);
    let recovered = a.or(b).unwrap();
    assert_eq!(recovered.id, c.id);
    assert_eq!(recovered.created_at, c.created_at);
    assert_eq!(recovered.execution_input, before.execution_input);
    assert_eq!(recovered.view.notes[0].name.as_deref(), Some("已确认保存"));
    assert_eq!(recovered.batches[0].commands.len(), 1);
    assert_eq!(recovered.batches[0].state, RoundState::Failed);
    assert_eq!(recovered.operations.len(), 2);
    assert_eq!(recovered.operations[0].actor.user_id(), Some(user));
    assert_eq!(recovered.operations[1].action, CandidateAction::Resume);
    assert!(
        matches!(&recovered.operations[1].actor,CandidateActor::System{id,display_name} if id=="curation-recovery" && display_name=="系统")
    );
    assert!(
        k.heartbeat_candidate(project, c.id, old_owner)
            .await
            .is_err()
    );
    assert!(
        k.open_candidate_batch(project, c.id, Some(old_owner), author())
            .await
            .is_err()
    );
    assert!(
        k.append_candidate(
            project,
            c.id,
            Some(old_owner),
            batch,
            &[note(c.base.endpoints[0], "迟到结果")]
        )
        .await
        .is_err()
    );
    assert!(
        k.checkpoint_candidate(
            project,
            c.id,
            CandidateState::Ready,
            json!({"run_id":old_owner})
        )
        .await
        .is_err()
    );
    assert_eq!(
        k.candidate(project, c.id).await.unwrap().view.notes[0]
            .name
            .as_deref(),
        Some("已确认保存")
    );
}

pub async fn manual_stop_wins_after_scan<S: KnowledgeStore + Clone>(store: S) {
    let (k, p, c, _, _) = seed(&store).await;
    assert!(scanned(&k).await.iter().any(|r| r.id == c.id));
    k.cancel_candidate(p, c.id).await.unwrap();
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    let saved = k.candidate(p, c.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert_eq!(saved.operations.len(), 1);
}

pub async fn renewed_heartbeat_wins_after_scan<S: KnowledgeStore + Clone>(store: S) {
    let (k, p, c, owner, _) = seed(&store).await;
    assert!(scanned(&k).await.iter().any(|r| r.id == c.id));
    k.heartbeat_candidate(p, c.id, owner).await.unwrap();
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        k.candidate(p, c.id).await.unwrap().progress["run_id"],
        json!(owner)
    );
}

pub async fn missing_input_and_changed_baseline_are_retained_and_blocked<
    S: KnowledgeStore + Clone,
>(
    store: S,
) {
    let (k, p, mut c, _, _) = seed(&store).await;
    c.execution_input = None;
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    let saved = k.candidate(p, c.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert_eq!(saved.progress["recovery_blocked"], true);
    assert!(
        saved.progress["message"]
            .as_str()
            .unwrap()
            .contains("恢复资料")
    );
    assert!(scanned(&k).await.iter().all(|r| r.id != c.id));
    let (k, p, c, _, _) = seed(&store).await;
    let round = k
        .begin(
            p,
            Author::Person {
                user: UserId::new(),
            },
        )
        .await
        .unwrap();
    k.apply(round, &[note(c.base.endpoints[0], "人工修改的正式文档")])
        .await
        .unwrap();
    k.finish(round, RoundState::Done, None).await.unwrap();
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    let saved = k.candidate(p, c.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert!(
        saved.progress["message"]
            .as_str()
            .unwrap()
            .contains("正式文档已变化")
    );
    let summary = k
        .candidates(p, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == c.id)
        .unwrap();
    assert_eq!(summary.progress["resume_available"], false);
    assert_eq!(
        k.endpoint(c.base.endpoints[0])
            .await
            .unwrap()
            .note
            .unwrap()
            .name
            .as_deref(),
        Some("人工修改的正式文档")
    );
}

pub async fn scan_pages_without_loading_or_resuming_closed_tasks<S: KnowledgeStore + Clone>(
    store: S,
) {
    let (k, p, c, _, _) = seed(&store).await;
    let mut tx = store.begin().await.unwrap();
    let mut closed = c.clone();
    closed.state = CandidateState::Ready;
    tx.put_candidate(&closed).await.unwrap();
    let prefix = uuid::Uuid::new_v4().as_u128() & !0xffffu128;
    for i in 1..=65 {
        let mut row = c.clone();
        row.id = uuid::Uuid::from_u128(prefix + i);
        tx.put_candidate(&row).await.unwrap();
    }
    for state in [
        CandidateState::Ready,
        CandidateState::Partial,
        CandidateState::Interrupted,
        CandidateState::Applied,
        CandidateState::Discarded,
    ] {
        let mut row = c.clone();
        row.id = uuid::Uuid::new_v4();
        row.state = state;
        tx.put_candidate(&row).await.unwrap();
    }
    tx.commit().await.unwrap();
    let first = k.recovery_candidates(None, 500).await.unwrap();
    assert_eq!(first.len(), 50);
    let selected: Vec<_> = scanned(&k)
        .await
        .into_iter()
        .filter(|r| r.project_id == p)
        .map(|r| r.id)
        .collect();
    assert_eq!(
        selected,
        (1..=65)
            .map(|i| uuid::Uuid::from_u128(prefix + i))
            .collect::<Vec<_>>()
    );
}
