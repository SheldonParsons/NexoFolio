use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, ProjectId, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::{Value, json};
mod folder_names;
pub use folder_names::folder_names;
#[derive(Clone)]
pub struct Facts(Vec<EndpointFacts>);
#[async_trait]
impl EndpointReader for Facts {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(self
            .0
            .iter()
            .filter(|e| e.summary.project_id == p)
            .map(|e| e.summary.clone())
            .collect())
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(self
            .0
            .iter()
            .find(|e| e.summary.id == id || e.aliases.contains(&id))
            .cloned())
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok(None)
    }
}
pub fn facts() -> Facts {
    let p = ProjectId::new();
    Facts(
        (0..2)
            .map(|i| EndpointFacts {
                summary: EndpointSummary {
                    id: EndpointId::new(),
                    project_id: p,
                    method: "POST".into(),
                    path_template: format!("/synthetic/{i}"),
                    external: false,
                    declared: false,
                    environments: vec![],
                },
                aliases: vec![],
                addresses: vec![],
                examples: vec![],
                requests: vec![],
                fields: vec![
                    field("data", FieldLocation::RequestBody),
                    field("data.id", FieldLocation::RequestBody),
                    field("database", FieldLocation::RequestBody),
                    field("data.id", FieldLocation::ResponseBody { status: 200 }),
                ],
            })
            .collect(),
    )
}
fn field(path: &str, location: FieldLocation) -> FieldFacts {
    FieldFacts {
        location,
        path: FieldPath(
            path.split('.')
                .map(|s| PathSegment::Key(s.into()))
                .collect(),
        ),
        types: vec![ValueType::Object],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }
}
fn actor() -> CandidateActor {
    CandidateActor::User {
        id: UserId::new(),
        display_name: Some("合成资料操作人".into()),
    }
}
fn change(target: LifecycleTarget, retired: bool, version: u64) -> LifecycleChange {
    LifecycleChange {
        target,
        retired,
        expected_version: version,
        reason: if retired {
            "合成资料已替代".into()
        } else {
            "恢复使用".into()
        },
    }
}
async fn ready(k: &dyn KnowledgeCandidates, p: ProjectId, c: &Candidate) {
    k.checkpoint_candidate(
        p,
        c.id,
        CandidateState::Ready,
        json!({"directory_completed":true}),
    )
    .await
    .unwrap();
}
pub async fn lifecycle<S: KnowledgeStore + Clone>(store: S) {
    let f = facts();
    let p = f.0[0].summary.project_id;
    let a = f.0[0].summary.id;
    let b = f.0[1].summary.id;
    let k = Knowledge::new(store.clone(), f);
    let user = actor();
    let root = FolderId::new();
    let child = FolderId::new();
    let round = k
        .begin(
            p,
            Author::Person {
                user: user.user_id().unwrap(),
            },
        )
        .await
        .unwrap();
    k.apply(
        round,
        &[
            Command::PutFolder {
                id: root,
                parent: None,
                name: "旧模块".into(),
                summary: Some("历史介绍".into()),
                position: 0,
            },
            Command::PutFolder {
                id: child,
                parent: Some(root),
                name: "子目录".into(),
                summary: None,
                position: 0,
            },
            Command::Place {
                endpoint: a,
                folder: child,
            },
            Command::Place {
                endpoint: b,
                folder: root,
            },
        ],
    )
    .await
    .unwrap();
    k.finish(round, RoundState::Done, None).await.unwrap();
    let state = k
        .change_lifecycle(
            p,
            change(LifecycleTarget::Folder { id: root }, true, 0),
            user.clone(),
        )
        .await
        .unwrap();
    assert!(state.folder(child).is_some());
    assert!(state.endpoint(a).is_none());
    assert_eq!(state.direct_count(), 1);
    // No immediate destructive cleanup; history remains readable.
    assert_eq!(
        k.endpoint(a).await.unwrap().placement.unwrap().folder,
        child
    );
    let c = k.create_candidate(p, None, None).await.unwrap();
    assert!(c.view.folders.is_empty());
    assert!(c.view.placements.is_empty());
    assert_eq!(c.view.endpoints.len(), 2);
    ready(&k, p, &c).await;
    k.apply_candidate(p, c.id, user.user_id().unwrap())
        .await
        .unwrap();
    assert!(k.endpoint(a).await.unwrap().placement.is_none());
    assert_eq!(k.catalogue(p).await.unwrap().folders.len(), 2);
    assert!(!k.lifecycle(p).await.unwrap().pending());
    let restored = k
        .change_lifecycle(
            p,
            change(LifecycleTarget::Folder { id: root }, false, 1),
            user.clone(),
        )
        .await
        .unwrap();
    assert!(restored.pending());
    assert!(restored.folder(child).is_none());
    let state = k
        .change_lifecycle(
            p,
            change(LifecycleTarget::Endpoint { id: a }, true, 2),
            user.clone(),
        )
        .await
        .unwrap();
    assert!(state.endpoint(a).is_some());
    let state = k
        .change_lifecycle(
            p,
            change(LifecycleTarget::Endpoint { id: b }, true, 3),
            user.clone(),
        )
        .await
        .unwrap();
    assert_eq!(state.version, 4);
    let c = k.create_candidate(p, None, None).await.unwrap();
    assert!(c.view.endpoints.is_empty());
    assert_eq!(
        c.view.folders.len(),
        2,
        "unretired manual directory content must not be silently removed"
    );
    ready(&k, p, &c).await;
    k.apply_candidate(p, c.id, user.user_id().unwrap())
        .await
        .unwrap();
    assert!(k.lifecycle(p).await.unwrap().endpoint(a).is_some());
    assert!(
        k.change_lifecycle(
            p,
            change(LifecycleTarget::Endpoint { id: a }, false, 3),
            user
        )
        .await
        .is_err()
    );
    assert_eq!(k.lifecycle(p).await.unwrap().version, 4);
}
pub async fn fields<S: KnowledgeStore + Clone>(store: S) {
    let f = facts();
    let p = f.0[0].summary.project_id;
    let a = f.0[0].summary.id;
    let b = f.0[1].summary.id;
    let k = Knowledge::new(store, f);
    let user = actor();
    let target = LifecycleTarget::Field {
        endpoint: a,
        location: FieldLocation::RequestBody,
        path: field("data", FieldLocation::RequestBody).path,
    };
    let s = k
        .change_lifecycle(p, change(target.clone(), true, 0), user.clone())
        .await
        .unwrap();
    assert!(s.field_key(a, "request_body:data.id").is_some());
    assert!(s.field_key(a, "request_body:database").is_none());
    assert!(s.field_key(a, "response_body:200:data.id").is_none());
    assert!(s.endpoint(a).is_none());
    let link = Link {
        id: nexofolio_common::LinkId::new(),
        from: b,
        from_field: None,
        to: a,
        to_field: Some("data.id".into()),
        relation: Relation::ParameterSource,
        evidence: "合成引用".into(),
        context: LinkContext {
            to_location: Some(FieldLocation::RequestBody),
            ..Default::default()
        },
        author: Author::Person {
            user: user.user_id().unwrap(),
        },
        at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    };
    assert!(s.link_retired(&link));
    let c = k.create_candidate(p, Some(b), None).await.unwrap();
    ready(&k, p, &c).await;
    k.apply_candidate(p, c.id, user.user_id().unwrap())
        .await
        .unwrap();
    assert!(
        k.lifecycle(p).await.unwrap().pending(),
        "single-interface work must not reconcile project lifecycle"
    );
    let s = k
        .change_lifecycle(
            p,
            change(LifecycleTarget::Endpoint { id: a }, true, 1),
            user.clone(),
        )
        .await
        .unwrap();
    assert_eq!(s.direct_count(), 2);
    k.change_lifecycle(
        p,
        change(LifecycleTarget::Endpoint { id: a }, false, 2),
        user.clone(),
    )
    .await
    .unwrap();
    assert!(
        k.lifecycle(p)
            .await
            .unwrap()
            .field_key(a, "request_body:data.id")
            .is_some(),
        "undo endpoint must retain direct field decision"
    );
    let s = k
        .change_lifecycle(p, change(target, false, 3), user)
        .await
        .unwrap();
    assert!(s.field_key(a, "request_body:data.id").is_none());
}
pub async fn stale<S: KnowledgeStore + Clone>(store: S) {
    let f = facts();
    let p = f.0[0].summary.project_id;
    let a = f.0[0].summary.id;
    let k = Knowledge::new(store.clone(), f);
    let user = actor();
    let c = k.create_candidate(p, None, None).await.unwrap();
    let owner = uuid::Uuid::new_v4();
    k.checkpoint_candidate(p, c.id, CandidateState::Running, json!({"run_id":owner}))
        .await
        .unwrap();
    k.freeze_candidate_input(p, c.id, owner, json!({"version":2}))
        .await
        .unwrap();
    k.change_lifecycle(
        p,
        change(LifecycleTarget::Endpoint { id: a }, true, 0),
        user.clone(),
    )
    .await
    .unwrap();
    let active = k.active_candidate(p).await.unwrap().unwrap();
    assert_eq!(active.progress["lifecycle_version"], 0);
    assert_eq!(active.progress["lifecycle_stale"], true);
    assert_eq!(active.progress["resume_available"], false);
    assert!(
        k.candidate(p, c.id)
            .await
            .unwrap()
            .progress
            .get("lifecycle_stale")
            .is_none()
    );
    assert_eq!(
        k.candidate(p, c.id).await.unwrap().state,
        CandidateState::Running,
        "retirement must not discard running work"
    );
    k.cancel_candidate(p, c.id).await.unwrap();
    assert_eq!(
        k.resume_candidate(p, c.id, uuid::Uuid::new_v4(), json!({"version":2}))
            .await
            .unwrap_err(),
        KnowledgeError::LifecycleStale
    );
    assert_eq!(
        k.create_candidate(p, None, Some(c.id)).await.unwrap_err(),
        KnowledgeError::CandidateStale
    );
    let mut old = k.candidate(p, c.id).await.unwrap();
    old.state = CandidateState::Running;
    old.updated_at = chrono::DateTime::from_timestamp(0, 0).unwrap();
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&old).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        k.recover_candidate(p, c.id, uuid::Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    let saved = k.candidate(p, c.id).await.unwrap();
    assert_eq!(saved.state, CandidateState::Interrupted);
    assert!(saved.execution_input.is_some());
    let mut tx = store.begin().await.unwrap();
    let mut old = saved;
    old.state = CandidateState::Ready;
    tx.put_candidate(&old).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        k.apply_candidate(p, c.id, user.user_id().unwrap())
            .await
            .unwrap_err(),
        KnowledgeError::LifecycleStale
    );
    assert!(k.lifecycle(p).await.unwrap().endpoint(a).is_some());
    for rows in [
        k.candidates(p, 30).await.unwrap(),
        k.entry_candidates(p, CandidateEntryKind::Panel, Some(c.id))
            .await
            .unwrap(),
        k.entry_candidates(p, CandidateEntryKind::Review, None)
            .await
            .unwrap(),
    ] {
        let old = rows.iter().find(|row| row.id == c.id).unwrap();
        assert_eq!(old.progress["lifecycle_stale"], true);
        assert_eq!(old.progress["resume_available"], false);
    }

    // Before the first real checkpoint, the frozen baseline already has the
    // correct version. A stored progress value must not override its identity.
    let fresh = k.create_candidate(p, None, None).await.unwrap();
    k.checkpoint_candidate(
        p,
        fresh.id,
        CandidateState::Running,
        json!({"lifecycle_version":999,"fields_updated":7}),
    )
    .await
    .unwrap();
    let rows = k
        .entry_candidates(p, CandidateEntryKind::Panel, Some(c.id))
        .await
        .unwrap();
    let current = rows.iter().find(|row| row.id == fresh.id).unwrap();
    assert_eq!(current.progress["lifecycle_version"], 1);
    assert_eq!(current.progress["lifecycle_stale"], false);
    assert_eq!(current.progress["fields_updated"], 7);
    assert_eq!(
        rows.iter().find(|row| row.id == c.id).unwrap().progress["lifecycle_stale"],
        true
    );
    assert_eq!(
        k.candidate(p, fresh.id).await.unwrap().progress["lifecycle_version"],
        999
    );
}

pub async fn rollback<S: KnowledgeStore + Clone>(store: S) {
    let f = facts();
    let p = f.0[0].summary.project_id;
    let a = f.0[0].summary.id;
    let b = f.0[1].summary.id;
    let k = Knowledge::new(store.clone(), f);
    let user = actor();
    let folder = FolderId::new();
    let r = k
        .begin(
            p,
            Author::Person {
                user: user.user_id().unwrap(),
            },
        )
        .await
        .unwrap();
    k.apply(
        r,
        &[
            Command::PutFolder {
                id: folder,
                parent: None,
                name: "合成旧目录".into(),
                summary: None,
                position: 0,
            },
            Command::Place {
                endpoint: a,
                folder,
            },
        ],
    )
    .await
    .unwrap();
    k.finish(r, RoundState::Done, None).await.unwrap();
    k.change_lifecycle(
        p,
        change(LifecycleTarget::Folder { id: folder }, true, 0),
        user.clone(),
    )
    .await
    .unwrap();
    let mut c = k.create_candidate(p, None, None).await.unwrap();
    c.state = CandidateState::Ready;
    c.batches.push(CandidateBatch {
        id: nexofolio_common::RoundId::new(),
        author: Author::Curate {
            pass: "fixture".into(),
            model: None,
        },
        state: RoundState::Done,
        commands: vec![
            Command::PutEndpointNote {
                endpoint: b,
                name: Some("must roll back".into()),
                purpose: None,
                context: Default::default(),
            },
            Command::Place {
                endpoint: b,
                folder: FolderId::new(),
            },
        ],
    });
    let mut tx = store.begin().await.unwrap();
    tx.put_candidate(&c).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        k.apply_candidate(p, c.id, user.user_id().unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        k.endpoint(a).await.unwrap().placement.unwrap().folder,
        folder
    );
    assert!(k.endpoint(b).await.unwrap().note.is_none());
    assert_eq!(k.rounds(p, 100).await.unwrap().len(), 1);
    let lifecycle = k.lifecycle(p).await.unwrap();
    assert_eq!(lifecycle.version, 1);
    assert_eq!(lifecycle.reconciled_version, 0);
    assert!(lifecycle.folder(folder).is_some());
}
impl Facts {
    #[allow(dead_code)]
    pub fn ids(&self) -> (ProjectId, EndpointId, EndpointId) {
        (
            self.0[0].summary.project_id,
            self.0[0].summary.id,
            self.0[1].summary.id,
        )
    }
}
