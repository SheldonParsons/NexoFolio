//! Real SQL transactions; isolated fixtures, never the live project.
#[path = "../../modules/knowledge/core/tests/candidates/support.rs"]
#[allow(dead_code)]
mod support;
use nexofolio_common::{FolderId, Secret, UserId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn progress_distinguishes_lease_renewal_and_frozen_input_from_work_progress() {
    let facts = support::facts();
    let project = facts.0.project_id;
    let knowledge = Knowledge::new(store().await, facts);
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4();
    let progress = json!({"run_id":owner,"status":"running","updated_at":"2026-10-09T00:00:00Z"});
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Running,
            progress.clone(),
        )
        .await
        .unwrap();
    knowledge
        .heartbeat_candidate(project, candidate.id, owner)
        .await
        .unwrap();
    let rows = knowledge
        .entry_candidates(project, CandidateEntryKind::Panel, None)
        .await
        .unwrap();
    assert_eq!(rows[0].progress["updated_at"], progress["updated_at"]);
    assert_ne!(rows[0].progress["heartbeat_at"], progress["updated_at"]);
    assert_eq!(rows[0].progress["resume_available"], false);
    knowledge
        .freeze_candidate_input(project, candidate.id, owner, json!({"version":2}))
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(project, candidate.id, CandidateState::Partial, progress)
        .await
        .unwrap();
    let rows = knowledge.candidates(project, 10).await.unwrap();
    assert_eq!(rows[0].progress["resume_available"], true);
}

async fn store() -> PostgresKnowledge {
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").expect("isolated database required"));
    let store = PostgresKnowledge::new(&url, 4, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    store
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn actual_sql_candidates_stay_isolated_and_apply_atomically_with_history() {
    support::lifecycle(store().await).await;
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn actual_sql_review_entry_finds_pending_rounds_beyond_the_history_page() {
    support::review_entry_selection(store().await).await;
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn actual_sql_panel_entry_orders_states_and_refreshes_scoped_selection() {
    support::panel_entry_selection(store().await).await;
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn late_apply_failure_rolls_back_every_official_write_and_review_decision() {
    let store = store().await;
    let facts = support::facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(store.clone(), facts);
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, support::author())
        .await
        .unwrap();
    knowledge
        .append_candidate(
            project,
            candidate.id,
            None,
            batch,
            &[support::note(endpoint, "候选说明")],
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
    // Simulate a stored proposal invalidated/corrupted after preview validation.
    let mut saved = knowledge.candidate(project, candidate.id).await.unwrap();
    saved.batches[0].commands.push(Command::PutFolder {
        id: FolderId::new(),
        parent: Some(FolderId::new()),
        name: "无效父级".into(),
        summary: None,
        position: 0,
    });
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&saved).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .apply_candidate(project, candidate.id, UserId::new())
            .await
            .unwrap_err(),
        KnowledgeError::NoSuchFolder
    );
    assert!(knowledge.notes(project).await.unwrap().is_empty());
    assert!(knowledge.rounds(project, 100).await.unwrap().is_empty());
    assert!(
        knowledge
            .catalogue(project)
            .await
            .unwrap()
            .folders
            .is_empty()
    );
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
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn persisted_candidates_survive_new_service_instances_and_reject_stale_manual_edits() {
    let store = store().await;
    let facts = support::facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(store.clone(), facts.clone());
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, support::author())
        .await
        .unwrap();
    knowledge
        .append_candidate(
            project,
            candidate.id,
            None,
            batch,
            &[support::note(endpoint, "候选说明")],
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
            CandidateState::Ready,
            json!({"status":"done","fields_updated":24}),
        )
        .await
        .unwrap();
    drop(knowledge);
    let recovered = Knowledge::new(store, facts);
    let list = recovered.candidates(project, 10).await.unwrap();
    assert_eq!(list[0].progress["fields_updated"], 24);
    assert_eq!(list[0].commands, 1);
    let user = UserId::new();
    let edit = recovered
        .begin(project, Author::Person { user })
        .await
        .unwrap();
    recovered
        .apply(edit, &[support::note(endpoint, "人工名称")])
        .await
        .unwrap();
    recovered
        .finish(edit, RoundState::Done, None)
        .await
        .unwrap();
    assert_eq!(
        recovered
            .apply_candidate(project, candidate.id, user)
            .await
            .unwrap_err(),
        KnowledgeError::CandidateStale
    );
    assert_eq!(
        recovered.notes(project).await.unwrap()[0].name.as_deref(),
        Some("人工名称")
    );
    assert_eq!(
        recovered
            .candidate(project, candidate.id)
            .await
            .unwrap()
            .state,
        CandidateState::Ready
    );
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn worker_writes_never_reload_or_rewrite_immutable_input() {
    let database = store().await;
    let facts = support::facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Knowledge::new(database.clone(), facts);
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4();
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Running,
            json!({"run_id":owner}),
        )
        .await
        .unwrap();
    let frozen = json!({"version":2,"fixture":"immutable".repeat(1_500_000)});
    knowledge
        .freeze_candidate_input(project, candidate.id, owner, frozen.clone())
        .await
        .unwrap();
    let mut tx = database.begin().await.unwrap();
    let projected = tx
        .candidate_projection(project, candidate.id)
        .await
        .unwrap()
        .unwrap();
    assert!(projected.execution_input.is_none());
    tx.commit().await.unwrap();
    let pool = sqlx::PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION knowledge.test_reject_input_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'immutable input touched by worker'; END $$").execute(&pool).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE TRIGGER test_reject_input_write BEFORE UPDATE OF execution_input ON knowledge.candidates FOR EACH ROW WHEN (OLD.id='{}'::uuid) EXECUTE FUNCTION knowledge.test_reject_input_write()",candidate.id))).execute(&pool).await.unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, Some(owner), support::author())
        .await
        .unwrap();
    let view = knowledge
        .append_candidate_view(
            project,
            candidate.id,
            Some(owner),
            batch,
            &[support::note(endpoint, "fixture accepted")],
        )
        .await
        .unwrap();
    assert_eq!(view.notes.len(), 1);
    assert!(
        knowledge
            .append_candidate_view(
                project,
                candidate.id,
                Some(uuid::Uuid::new_v4()),
                batch,
                &[support::note(endpoint, "obsolete worker")]
            )
            .await
            .is_err()
    );
    knowledge
        .close_candidate_batch(project, candidate.id, Some(owner), batch, RoundState::Done)
        .await
        .unwrap();
    let after = knowledge.candidate(project, candidate.id).await.unwrap();
    assert_eq!(after.execution_input, Some(frozen));
    assert_eq!(
        after.view.notes[0].name.as_deref(),
        Some("fixture accepted")
    );
    sqlx::query("DROP TRIGGER test_reject_input_write ON knowledge.candidates")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION knowledge.test_reject_input_write()")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn batched_fields_preserve_repeated_revisions_manual_ownership_and_rollback() {
    let store = store().await;
    let facts = support::facts();
    let (project, endpoint) = (facts.0.project_id, facts.0.id);
    let knowledge = Knowledge::new(store.clone(), facts);
    let user = UserId::new();
    let at = "2026-10-14T00:00:00Z".parse().unwrap();
    let manual = FieldNote {
        endpoint,
        path: "manual".into(),
        text: "keep".into(),
        values: vec![],
        context: Default::default(),
        author: Author::Person { user },
        at,
    };
    let original = FieldNote {
        path: "repeat".into(),
        text: "original".into(),
        author: support::author(),
        ..manual.clone()
    };
    let mut tx = store.begin().await.unwrap();
    tx.put_field_note(project, &manual).await.unwrap();
    tx.put_field_note(project, &original).await.unwrap();
    tx.commit().await.unwrap();
    let field = |path: &str, text: &str| Command::PutFieldNote {
        endpoint,
        path: path.into(),
        text: text.into(),
        values: vec![],
        context: Default::default(),
    };
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let batch = knowledge
        .open_candidate_batch(project, candidate.id, None, support::author())
        .await
        .unwrap();
    let mut commands = vec![
        field("manual", "keep"),
        field("repeat", "one"),
        field("repeat", "two"),
        Command::RemoveFieldNote {
            endpoint,
            path: "repeat".into(),
        },
        field("repeat", "three"),
    ];
    commands.extend((0..105).map(|i| field(&format!("field-{i}"), "generated")));
    knowledge
        .append_candidate(project, candidate.id, None, batch, &commands)
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
    let rounds = knowledge
        .apply_candidate(project, candidate.id, user)
        .await
        .unwrap();
    let mut tx = store.begin_read().await.unwrap();
    assert_eq!(
        tx.field_note(endpoint, "manual").await.unwrap(),
        Some(manual.clone())
    );
    assert_eq!(
        tx.field_note(endpoint, "repeat")
            .await
            .unwrap()
            .unwrap()
            .text,
        "three"
    );
    let revisions = tx.revisions(rounds[0]).await.unwrap();
    assert_eq!(revisions.len(), 109); // The identical human-owned write is a no-op.
    let repeated: Vec<_> = revisions
        .iter()
        .filter(|r| matches!(&r.target,Target::FieldNote{path,..} if path=="repeat"))
        .collect();
    assert_eq!(
        repeated.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![4, 3, 2, 1]
    );
    assert_eq!(repeated[0].before, None);
    assert_eq!(repeated[1].before.as_ref().unwrap()["text"], "two");
    assert_eq!(repeated[2].before.as_ref().unwrap()["text"], "one");
    assert_eq!(repeated[3].before.as_ref().unwrap()["text"], "original");
    drop(tx);
    assert_eq!(
        knowledge
            .apply_candidate(project, candidate.id, user)
            .await
            .unwrap(),
        rounds
    );
    knowledge.roll_back(rounds[0]).await.unwrap();
    let mut tx = store.begin_read().await.unwrap();
    assert_eq!(
        tx.field_note(endpoint, "manual").await.unwrap(),
        Some(manual)
    );
    assert_eq!(
        tx.field_note(endpoint, "repeat").await.unwrap(),
        Some(original)
    );
    assert_eq!(tx.field_notes(endpoint).await.unwrap().len(), 2);
}
