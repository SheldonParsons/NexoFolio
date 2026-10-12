#[path = "lifecycle/support.rs"]
mod support;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
#[tokio::test]
async fn names_are_unique_only_among_effective_folders_with_safe_undo_and_rollback() {
    support::folder_names(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn retirement_projection_undo_and_versions() {
    support::lifecycle(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn retired_sources_and_object_paths_are_precise() {
    support::fields(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn old_candidates_resume_restart_and_inheritance_are_fenced() {
    support::stale(InMemoryKnowledge::new()).await;
}
#[tokio::test]
async fn failed_application_rolls_back_cleanup_and_preserves_immediate_retirement() {
    support::rollback(InMemoryKnowledge::new()).await;
}

#[tokio::test]
async fn alias_merge_cannot_implicitly_restore_retirement() {
    use async_trait::async_trait;
    use nexofolio_common::{EndpointId, ProjectId, UserId};
    use nexofolio_contracts::{endpoint::*, knowledge::*};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Aliased {
        inner: support::Facts,
        a: EndpointId,
        b: EndpointId,
        merged: Arc<AtomicBool>,
    }
    #[async_trait]
    impl EndpointReader for Aliased {
        async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
            let mut values = self.inner.list(p).await?;
            if self.merged.load(Ordering::SeqCst) {
                values.retain(|e| e.id != self.a);
            }
            Ok(values)
        }
        async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
            if self.merged.load(Ordering::SeqCst) && (id == self.a || id == self.b) {
                let mut f = self.inner.get(self.b).await?.unwrap();
                f.aliases.push(self.a);
                Ok(Some(f))
            } else {
                self.inner.get(id).await
            }
        }
        async fn example(
            &self,
            _: EndpointId,
            _: &str,
        ) -> Result<Option<serde_json::Value>, EndpointError> {
            Ok(None)
        }
    }
    // Obtain project identity from the fixture's lightweight accessor.
    let inner = support::facts();
    let (p, a, b) = inner.ids();
    let merged = Arc::new(AtomicBool::new(false));
    let k = nexofolio_knowledge::Knowledge::new(
        InMemoryKnowledge::new(),
        Aliased {
            inner,
            a,
            b,
            merged: merged.clone(),
        },
    );
    let actor = CandidateActor::User {
        id: UserId::new(),
        display_name: None,
    };
    k.change_lifecycle(
        p,
        LifecycleChange {
            target: LifecycleTarget::Endpoint { id: a },
            retired: true,
            reason: "旧接口".into(),
            expected_version: 0,
        },
        actor.clone(),
    )
    .await
    .unwrap();
    merged.store(true, Ordering::SeqCst);
    assert!(k.lifecycle(p).await.unwrap().endpoint(b).is_some());
    let c = k.create_candidate(p, None, None).await.unwrap();
    assert!(c.view.endpoints.is_empty());
    k.change_lifecycle(
        p,
        LifecycleChange {
            target: LifecycleTarget::Endpoint { id: a },
            retired: false,
            reason: "恢复".into(),
            expected_version: 1,
        },
        actor,
    )
    .await
    .unwrap();
    assert!(k.lifecycle(p).await.unwrap().endpoint(b).is_none());
}

#[tokio::test]
async fn model_candidate_writes_cannot_reintroduce_a_retired_interface_note() {
    use nexofolio_contracts::knowledge::*;
    let facts = support::facts();
    let (p, a, b) = facts.ids();
    let user = nexofolio_common::UserId::new();
    let k = nexofolio_knowledge::Knowledge::new(InMemoryKnowledge::new(), facts);
    k.change_lifecycle(
        p,
        LifecycleChange {
            target: LifecycleTarget::Endpoint { id: a },
            retired: true,
            reason: "fixture".into(),
            expected_version: 0,
        },
        CandidateActor::User {
            id: user,
            display_name: None,
        },
    )
    .await
    .unwrap();
    let c = k.create_candidate(p, None, None).await.unwrap();
    let batch = k
        .open_candidate_batch(
            p,
            c.id,
            None,
            Author::Curate {
                pass: "fixture".into(),
                model: None,
            },
        )
        .await
        .unwrap();
    let note = |endpoint| Command::PutEndpointNote {
        endpoint,
        name: Some("must not publish".into()),
        purpose: None,
        context: Default::default(),
    };
    let result = k
        .append_candidate(p, c.id, None, batch, &[note(b), note(a)])
        .await;
    assert!(matches!(result, Err(KnowledgeError::InvalidLifecycle(_))));
    let saved = k.candidate(p, c.id).await.unwrap();
    assert!(saved.view.notes.is_empty());
    assert!(saved.batches[0].commands.is_empty());
}
