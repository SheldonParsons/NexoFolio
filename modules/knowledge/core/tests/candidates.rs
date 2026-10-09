use nexofolio_common::{EndpointId, ProjectId, UserId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::json;

#[path = "candidates/support.rs"]
mod support;
use support::*;

#[tokio::test]
async fn generated_proposals_are_isolated_until_atomic_idempotent_application() {
    lifecycle(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn panel_entry_respects_state_priority_and_scoped_selection() {
    panel_entry_selection(InMemoryKnowledge::new()).await;
}

#[tokio::test]
async fn human_changes_make_a_candidate_stale_without_partial_publication() {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), facts);
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, author())
        .await
        .unwrap();
    knowledge
        .append_candidate(
            project,
            candidate.id,
            None,
            batch,
            &[note(endpoint, "模型名称")],
        )
        .await
        .unwrap();
    knowledge
        .close_candidate_batch(project, candidate.id, None, batch, RoundState::Done)
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(project, candidate.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    let user = UserId::new();
    let edit = knowledge
        .begin(project, Author::Person { user })
        .await
        .unwrap();
    knowledge
        .apply(edit, &[note(endpoint, "人工名称")])
        .await
        .unwrap();
    knowledge
        .finish(edit, RoundState::Done, None)
        .await
        .unwrap();
    assert_eq!(
        knowledge
            .apply_candidate(project, candidate.id, user)
            .await
            .unwrap_err(),
        KnowledgeError::CandidateStale
    );
    assert_eq!(
        knowledge
            .endpoint(endpoint)
            .await
            .unwrap()
            .note
            .unwrap()
            .name
            .as_deref(),
        Some("人工名称")
    );
    assert_eq!(knowledge.rounds(project, 100).await.unwrap().len(), 1);
    assert_eq!(
        knowledge
            .candidate(project, candidate.id)
            .await
            .unwrap()
            .state,
        CandidateState::Ready
    );
}

#[tokio::test]
async fn invalid_batches_do_not_mutate_a_draft_and_discard_never_mutates_official_knowledge() {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), facts);
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, author())
        .await
        .unwrap();
    assert_eq!(
        knowledge
            .append_candidate(
                project,
                candidate.id,
                None,
                batch,
                &[
                    note(endpoint, "合法项"),
                    note(EndpointId::new(), "外项目项")
                ]
            )
            .await
            .unwrap_err(),
        KnowledgeError::WrongProject
    );
    assert!(
        knowledge
            .candidate(project, candidate.id)
            .await
            .unwrap()
            .view
            .notes
            .is_empty()
    );
    knowledge
        .append_candidate(
            project,
            candidate.id,
            None,
            batch,
            &[note(endpoint, "候选名称")],
        )
        .await
        .unwrap();
    knowledge
        .close_candidate_batch(project, candidate.id, None, batch, RoundState::Done)
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(project, candidate.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    let user = UserId::new();
    knowledge
        .discard_candidate(project, candidate.id, user)
        .await
        .unwrap();
    knowledge
        .discard_candidate(project, candidate.id, user)
        .await
        .unwrap();
    assert!(knowledge.endpoint(endpoint).await.unwrap().note.is_none());
    assert_eq!(
        knowledge
            .apply_candidate(project, candidate.id, user)
            .await
            .unwrap_err(),
        KnowledgeError::CandidateClosed
    );
    assert!(
        knowledge
            .candidate(ProjectId::new(), candidate.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn interrupted_proposals_resume_acknowledged_output_without_publishing_or_losing_authorship()
{
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), facts);
    let old = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, old.id, None, author())
        .await
        .unwrap();
    knowledge
        .append_candidate(
            project,
            old.id,
            None,
            batch,
            &[note(endpoint, "已完成说明")],
        )
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(
            project,
            old.id,
            CandidateState::Interrupted,
            json!({"status":"failed"}),
        )
        .await
        .unwrap();
    let resumed = knowledge
        .create_candidate(project, None, Some(old.id))
        .await
        .unwrap();
    assert_eq!(resumed.view.notes[0].name.as_deref(), Some("已完成说明"));
    assert_eq!(resumed.batches[0].state, RoundState::Done);
    assert_eq!(resumed.batches[0].author, author());
    assert_eq!(resumed.parent, Some(old.id));
    assert!(knowledge.notes(project).await.unwrap().is_empty());
}

#[tokio::test]
async fn generated_notes_cannot_overwrite_preexisting_human_notes() {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), facts);
    let manual = knowledge
        .begin(
            project,
            Author::Person {
                user: UserId::new(),
            },
        )
        .await
        .unwrap();
    knowledge
        .apply(manual, &[note(endpoint, "人工名称")])
        .await
        .unwrap();
    knowledge
        .finish(manual, RoundState::Done, None)
        .await
        .unwrap();
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, author())
        .await
        .unwrap();
    assert_eq!(
        knowledge
            .append_candidate(
                project,
                candidate.id,
                None,
                batch,
                &[note(endpoint, "模型覆盖")]
            )
            .await
            .unwrap_err(),
        KnowledgeError::ProtectedKnowledge
    );
    assert_eq!(
        knowledge
            .candidate(project, candidate.id)
            .await
            .unwrap()
            .view
            .notes[0]
            .name
            .as_deref(),
        Some("人工名称")
    );
}
