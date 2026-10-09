use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, ProjectId, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::{CandidateSession, Knowledge};
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct Facts(pub EndpointSummary);
#[async_trait]
impl EndpointReader for Facts {
    async fn list(&self, project: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(if project == self.0.project_id {
            vec![self.0.clone()]
        } else {
            vec![]
        })
    }
    async fn get(&self, _: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(None)
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok(None)
    }
}
pub fn facts() -> Facts {
    Facts(EndpointSummary {
        id: EndpointId::new(),
        project_id: ProjectId::new(),
        method: "GET".into(),
        path_template: "/fixture/detail/{id}".into(),
        external: false,
        declared: false,
        environments: vec![],
    })
}
pub fn author() -> Author {
    Author::Curate {
        pass: "fixture".into(),
        model: Some("fixed".into()),
    }
}
pub fn note(endpoint: EndpointId, name: &str) -> Command {
    Command::PutEndpointNote {
        endpoint,
        name: Some(name.into()),
        purpose: Some("查询记录详情".into()),
        context: Default::default(),
    }
}

/// An older pending round must remain discoverable after the history page fills up.
#[allow(dead_code)] // Shared by independent in-memory and PostgreSQL test binaries.
pub async fn review_entry_selection<S: KnowledgeStore + Clone + 'static>(store: S) {
    let facts = facts();
    let project = facts.0.project_id;
    let knowledge = Knowledge::new(store.clone(), facts);
    let mut first = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    first.state = CandidateState::Ready;
    let mut partial = first.clone();
    partial.id = uuid::Uuid::new_v4();
    partial.state = CandidateState::Partial;
    partial.created_at += chrono::Duration::minutes(1);
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&first).await.unwrap();
    tx.put_candidate(&partial).await.unwrap();
    let mut newest = first.clone();
    for i in 2..62 {
        newest.id = uuid::Uuid::new_v4();
        newest.state = CandidateState::Applied;
        newest.created_at = first.created_at + chrono::Duration::minutes(i);
        tx.put_candidate(&newest).await.unwrap();
    }
    let mut foreign = partial.clone();
    foreign.id = uuid::Uuid::new_v4();
    foreign.project_id = ProjectId::new();
    foreign.created_at += chrono::Duration::days(1);
    tx.put_candidate(&foreign).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        knowledge
            .candidates(project, 50)
            .await
            .unwrap()
            .iter()
            .all(|c| c.state == CandidateState::Applied)
    );
    let entry = knowledge
        .entry_candidates(project, CandidateEntryKind::Review, None)
        .await
        .unwrap();
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].id, partial.id);

    first.state = CandidateState::Discarded;
    partial.state = CandidateState::Interrupted;
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&first).await.unwrap();
    tx.put_candidate(&partial).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Review, None)
            .await
            .unwrap()[0]
            .id,
        newest.id
    );
    assert!(
        knowledge
            .entry_candidates(ProjectId::new(), CandidateEntryKind::Review, None)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Shared with PostgreSQL integration tests: this checks actual proposal semantics.
pub async fn panel_entry_selection<S: KnowledgeStore + Clone + 'static>(store: S) {
    let facts = facts();
    let project = facts.0.project_id;
    let knowledge = Knowledge::new(store.clone(), facts);
    let running = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let mut ready = running.clone();
    ready.id = uuid::Uuid::new_v4();
    ready.state = CandidateState::Ready;
    ready.created_at += chrono::Duration::minutes(1);
    let mut interrupted = ready.clone();
    interrupted.id = uuid::Uuid::new_v4();
    interrupted.state = CandidateState::Interrupted;
    interrupted.created_at += chrono::Duration::minutes(1);
    let mut applied = interrupted.clone();
    applied.id = uuid::Uuid::new_v4();
    applied.state = CandidateState::Applied;
    applied.created_at += chrono::Duration::minutes(1);
    let mut tx = store.begin().await.unwrap();
    for c in [&ready, &interrupted, &applied] {
        tx.put_candidate(c).await.unwrap();
    }
    let mut latest = applied.clone();
    latest.state = CandidateState::Discarded;
    for i in 4..64 {
        latest.id = uuid::Uuid::new_v4();
        latest.created_at = running.created_at + chrono::Duration::minutes(i);
        tx.put_candidate(&latest).await.unwrap();
    }
    let mut foreign = running.clone();
    foreign.id = uuid::Uuid::new_v4();
    foreign.project_id = ProjectId::new();
    tx.put_candidate(&foreign).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Panel, None)
            .await
            .unwrap()[0]
            .id,
        running.id
    );
    let shown = knowledge
        .entry_candidates(project, CandidateEntryKind::Panel, Some(applied.id))
        .await
        .unwrap();
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].id, running.id);
    assert_eq!(shown[1].id, applied.id);
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Panel, Some(foreign.id))
            .await
            .unwrap()
            .len(),
        1
    );
    let mut closed = running.clone();
    closed.state = CandidateState::Discarded;
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&closed).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Panel, None)
            .await
            .unwrap()[0]
            .id,
        interrupted.id
    );
    let mut tx = store.begin().await.unwrap();
    ready.state = CandidateState::Discarded;
    interrupted.state = CandidateState::Discarded;
    tx.put_candidate(&ready).await.unwrap();
    tx.put_candidate(&interrupted).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Panel, None)
            .await
            .unwrap()[0]
            .id,
        applied.id
    );
    let mut tx = store.begin().await.unwrap();
    applied.state = CandidateState::Discarded;
    tx.put_candidate(&applied).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        knowledge
            .entry_candidates(project, CandidateEntryKind::Panel, None)
            .await
            .unwrap()[0]
            .id,
        latest.id
    );
    assert!(
        knowledge
            .entry_candidates(
                ProjectId::new(),
                CandidateEntryKind::Panel,
                Some(running.id)
            )
            .await
            .unwrap()
            .is_empty()
    );
}

/// Shared with PostgreSQL integration tests: this checks actual proposal semantics.
pub async fn lifecycle<S: KnowledgeStore + 'static>(store: S) {
    let facts = facts();
    let project = facts.0.project_id;
    let endpoint = facts.0.id;
    let knowledge = Arc::new(Knowledge::new(store, facts));
    let candidate = knowledge
        .create_candidate(project, None, None)
        .await
        .unwrap();
    let session = CandidateSession::new(&candidate, knowledge.clone());
    let batch = session.begin(project, author()).await.unwrap();
    let root = FolderId::new();
    let leaf = FolderId::new();
    let commands = vec![
        note(endpoint, "查询业务记录"),
        Command::PutFolder {
            id: root,
            parent: None,
            name: "业务模块".into(),
            summary: Some("业务入口".into()),
            position: 0,
        },
        Command::PutFolder {
            id: leaf,
            parent: Some(root),
            name: "查询详情".into(),
            summary: Some("查看记录".into()),
            position: 0,
        },
        Command::Place {
            endpoint,
            folder: leaf,
        },
        Command::PutFieldNote {
            endpoint,
            path: "response:200/id".into(),
            text: "记录标识，具体范围待确认".into(),
            values: vec![],
            context: Default::default(),
        },
    ];
    session.apply(batch, &commands).await.unwrap();
    session.finish(batch, RoundState::Done, None).await.unwrap();
    assert_eq!(session.catalogue(project).await.unwrap().folders.len(), 2);
    assert_eq!(session.endpoint(endpoint).await.unwrap().fields.len(), 1);
    assert!(
        knowledge.notes(project).await.unwrap().is_empty(),
        "generation must not publish"
    );
    assert!(
        knowledge
            .catalogue(project)
            .await
            .unwrap()
            .folders
            .is_empty()
    );
    assert!(
        knowledge.rounds(project, 100).await.unwrap().is_empty(),
        "drafts must not enter official history"
    );
    knowledge
        .checkpoint_candidate(
            project,
            candidate.id,
            CandidateState::Ready,
            json!({"status":"done"}),
        )
        .await
        .unwrap();
    let reviewer = UserId::new();
    let rounds = knowledge
        .apply_candidate(project, candidate.id, reviewer)
        .await
        .unwrap();
    assert_eq!(rounds.len(), 1);
    let applied = knowledge.endpoint(endpoint).await.unwrap();
    assert_eq!(applied.note.unwrap().name.as_deref(), Some("查询业务记录"));
    assert_eq!(applied.fields.len(), 1);
    assert_eq!(
        applied.path.iter().map(|f| f.id).collect::<Vec<_>>(),
        vec![root, leaf]
    );
    assert_eq!(
        knowledge
            .apply_candidate(project, candidate.id, reviewer)
            .await
            .unwrap(),
        rounds,
        "repeated application must be idempotent"
    );
    assert_eq!(knowledge.rounds(project, 100).await.unwrap().len(), 1);
    let saved = knowledge.candidate(project, candidate.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Applied);
    assert_eq!(saved.reviewed_by, Some(reviewer));
    knowledge.roll_back(rounds[0]).await.unwrap();
    assert!(knowledge.endpoint(endpoint).await.unwrap().note.is_none());
    assert!(
        knowledge
            .catalogue(project)
            .await
            .unwrap()
            .folders
            .is_empty()
    );
}
