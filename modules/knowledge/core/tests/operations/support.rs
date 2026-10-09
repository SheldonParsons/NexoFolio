#[path = "../candidates/support.rs"]
#[allow(dead_code)]
mod fixtures;
use nexofolio_common::UserId;
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::json;

pub async fn lifecycle<S: KnowledgeStore + Clone + 'static>(store: S) {
    let facts = fixtures::facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(store, facts);
    let alice = CandidateActor::User {
        id: UserId::new(),
        display_name: Some("Alice".into()),
    };
    let bob = CandidateActor::User {
        id: UserId::new(),
        display_name: Some("Bob".into()),
    };
    let system = CandidateActor::System {
        id: "system".into(),
        display_name: "系统".into(),
    };
    let candidate = knowledge
        .create_candidate_as(project, None, None, alice.clone())
        .await
        .unwrap();
    assert_eq!(candidate.operations[0].actor, alice);
    assert_eq!(candidate.operations[0].at, candidate.created_at);
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, fixtures::author())
        .await
        .unwrap();
    knowledge
        .append_candidate(
            project,
            candidate.id,
            None,
            batch,
            &[fixtures::note(endpoint, "候选")],
        )
        .await
        .unwrap();
    knowledge
        .close_candidate_batch(project, candidate.id, None, batch, RoundState::Done)
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Partial,
            json!({"status":"failed"}),
        )
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4();
    let resumed = knowledge
        .resume_candidate_as(project, candidate.id, owner, json!({}), bob.clone())
        .await
        .unwrap();
    assert_eq!(resumed.created_at, candidate.created_at);
    assert_eq!(resumed.operations[0].actor, alice);
    assert_eq!(resumed.operations[1].action, CandidateAction::Resume);
    assert_eq!(resumed.operations[1].actor, bob);
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Ready,
            json!({"status":"done", "run_id":owner}),
        )
        .await
        .unwrap();
    knowledge
        .apply_candidate_as(project, candidate.id, system.clone())
        .await
        .unwrap();
    knowledge
        .apply_candidate_as(project, candidate.id, alice.clone())
        .await
        .unwrap();
    let applied = knowledge
        .candidate_operations(project, candidate.id)
        .await
        .unwrap();
    assert_eq!(applied.operations.len(), 3);
    assert_eq!(applied.operations[2].actor, system);
    assert_eq!(applied.reviewed_by, None);
    assert_eq!(applied.operations[2].at, applied.updated_at);

    let second = knowledge
        .create_candidate_as(project, None, None, system.clone())
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(project, second.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    knowledge
        .discard_candidate_as(project, second.id, bob.clone())
        .await
        .unwrap();
    knowledge
        .discard_candidate_as(project, second.id, alice.clone())
        .await
        .unwrap();
    let discarded = knowledge
        .candidate_operations(project, second.id)
        .await
        .unwrap();
    assert_eq!(discarded.operations.len(), 2);
    assert_eq!(discarded.operations[0].actor, system);
    assert_eq!(discarded.operations[1].actor, bob);
    assert_eq!(discarded.reviewed_by, bob.user_id());
    assert_eq!(
        knowledge
            .candidate_operations(nexofolio_common::ProjectId::new(), second.id)
            .await
            .unwrap_err(),
        KnowledgeError::NoSuchCandidate
    );

    let stale = knowledge
        .create_candidate_as(project, None, None, alice.clone())
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(project, stale.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    let edit = knowledge
        .begin(
            project,
            Author::Person {
                user: alice.user_id().unwrap(),
            },
        )
        .await
        .unwrap();
    knowledge
        .apply(edit, &[fixtures::note(endpoint, "人工修改")])
        .await
        .unwrap();
    knowledge
        .finish(edit, RoundState::Done, None)
        .await
        .unwrap();
    assert_eq!(
        knowledge
            .apply_candidate_as(project, stale.id, bob)
            .await
            .unwrap_err(),
        KnowledgeError::CandidateStale
    );
    assert_eq!(
        knowledge
            .candidate_operations(project, stale.id)
            .await
            .unwrap()
            .operations
            .len(),
        1
    );
}
