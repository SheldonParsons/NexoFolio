use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Facts(Arc<Mutex<Vec<EndpointFacts>>>);
#[async_trait]
impl EndpointReader for Facts {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.summary.project_id == p)
            .map(|f| f.summary.clone())
            .collect())
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.summary.id == id)
            .cloned())
    }
    async fn example(
        &self,
        _: EndpointId,
        _: &str,
    ) -> Result<Option<serde_json::Value>, EndpointError> {
        Ok(None)
    }
}
fn facts(p: ProjectId) -> EndpointFacts {
    EndpointFacts {
        requests: vec![],
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: p,
            method: "GET".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        fields: vec![],
        aliases: vec![],
        addresses: vec![],
        examples: vec![],
    }
}
fn note(id: EndpointId, name: &str) -> Command {
    Command::PutEndpointNote {
        endpoint: id,
        name: Some(name.into()),
        purpose: Some("有依据的用途".into()),
        context: Default::default(),
    }
}

#[tokio::test]
async fn added_observed_endpoints_inherit_drafts_but_deletions_and_human_changes_remain_protected()
{
    let p = ProjectId::new();
    let original = facts(p);
    let source = Facts(Arc::new(Mutex::new(vec![original.clone()])));
    let knowledge = Knowledge::new(InMemoryKnowledge::new(), source.clone());
    let draft = knowledge.create_candidate(p, None, None).await.unwrap();
    let batch = knowledge
        .open_candidate_batch(
            p,
            draft.id,
            None,
            Author::Curate {
                pass: "fixture".into(),
                model: None,
            },
        )
        .await
        .unwrap();
    knowledge
        .append_candidate(
            p,
            draft.id,
            None,
            batch,
            &[note(original.summary.id, "已整理名称")],
        )
        .await
        .unwrap();
    knowledge
        .close_candidate_batch(p, draft.id, None, batch, RoundState::Done)
        .await
        .unwrap();
    knowledge
        .checkpoint_candidate(p, draft.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    source.0.lock().unwrap().push(facts(p));
    let next = knowledge
        .create_candidate(p, None, Some(draft.id))
        .await
        .unwrap();
    assert_eq!(next.parent, Some(draft.id));
    assert_eq!(next.view.endpoints.len(), 2);
    assert_eq!(next.view.notes[0].name.as_deref(), Some("已整理名称"));
    assert_ne!(next.base_hash, draft.base_hash);
    assert!(knowledge.notes(p).await.unwrap().is_empty());
    assert_eq!(
        knowledge.candidate(p, draft.id).await.unwrap().state,
        CandidateState::Ready
    );
    knowledge
        .checkpoint_candidate(p, next.id, CandidateState::Ready, json!({}))
        .await
        .unwrap();
    source
        .0
        .lock()
        .unwrap()
        .retain(|f| f.summary.id != original.summary.id);
    assert!(matches!(
        knowledge.create_candidate(p, None, Some(draft.id)).await,
        Err(KnowledgeError::CandidateStale)
    ));
    source.0.lock().unwrap().push(original.clone());
    let user = UserId::new();
    let round = knowledge.begin(p, Author::Person { user }).await.unwrap();
    knowledge
        .apply(round, &[note(original.summary.id, "人工名称")])
        .await
        .unwrap();
    knowledge
        .finish(round, RoundState::Done, None)
        .await
        .unwrap();
    assert!(matches!(
        knowledge.create_candidate(p, None, Some(draft.id)).await,
        Err(KnowledgeError::CandidateStale)
    ));
    assert!(matches!(
        knowledge.apply_candidate(p, next.id, user).await,
        Err(KnowledgeError::CandidateStale)
    ));
    source
        .0
        .lock()
        .unwrap()
        .retain(|f| f.summary.id != original.summary.id);
    assert!(matches!(
        knowledge.create_candidate(p, None, Some(draft.id)).await,
        Err(KnowledgeError::CandidateStale)
    ));
}
