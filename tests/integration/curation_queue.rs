//! Real PostgreSQL admission/fencing tests, isolated from user data and paid models.
use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId, Secret, UserId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[allow(dead_code)]
#[path = "curation_restart/fixture.rs"]
mod fixture;

#[derive(Clone)]
struct Facts {
    rows: Vec<EndpointSummary>,
    alias: EndpointId,
    lists: Arc<AtomicUsize>,
}
#[async_trait]
impl EndpointReader for Facts {
    async fn identity(&self, id: EndpointId) -> Result<Option<EndpointIdentity>, EndpointError> {
        Ok(self
            .rows
            .iter()
            .find(|r| r.id == id || (id == self.alias && r.id == self.rows[0].id))
            .map(|r| EndpointIdentity {
                id: r.id,
                project_id: r.project_id,
                aliases: vec![self.alias],
            }))
    }
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .rows
            .iter()
            .filter(|r| r.project_id == p)
            .cloned()
            .collect())
    }
    async fn get(&self, _: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok(None)
    }
    async fn example(&self, _: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok(None)
    }
}
fn actor() -> CandidateActor {
    CandidateActor::User {
        id: UserId::new(),
        display_name: None,
    }
}
fn author() -> Author {
    Author::Curate {
        pass: "fixture".into(),
        model: None,
    }
}
fn facts() -> Facts {
    let a = ProjectId::new();
    let b = ProjectId::new();
    Facts {
        rows: [a, a, b]
            .into_iter()
            .map(|project_id| EndpointSummary {
                id: EndpointId::new(),
                project_id,
                method: "GET".into(),
                path_template: "/fixture".into(),
                external: false,
                declared: false,
                environments: vec![],
            })
            .collect(),
        alias: EndpointId::new(),
        lists: Arc::new(AtomicUsize::new(0)),
    }
}
async fn expire(store: &PostgresKnowledge, t: &CurationTask) {
    let mut t = t.clone();
    t.lease_until = Some(chrono::Utc::now() - chrono::TimeDelta::seconds(1));
    let mut tx = store.begin().await.unwrap();
    tx.lock_project(t.project_id).await.unwrap();
    tx.put_task(&t).await.unwrap();
    tx.commit().await.unwrap();
}
#[tokio::test]
#[ignore = "isolated TEST_DATABASE_URL required"]
async fn admission_fairness_fencing_and_decisions() {
    let db = fixture::database_url();
    let store = PostgresKnowledge::new(&Secret::new(db), 16, Duration::from_secs(5)).unwrap();
    store.migrate().await.unwrap();
    let initial_facts = facts();
    let p = initial_facts.rows[0].project_id;
    let q = initial_facts.rows[2].project_id;
    let e = initial_facts.rows[0].id;
    let k = Arc::new(Knowledge::new(store.clone(), initial_facts.clone()));
    let mut submissions = tokio::task::JoinSet::new();
    for n in 0..24 {
        let k = k.clone();
        let endpoint = if n % 2 == 0 { e } else { initial_facts.alias };
        submissions.spawn(async move {
            k.submit_task(p, Some(endpoint), None, None, actor())
                .await
                .unwrap()
        });
    }
    let mut tasks = vec![];
    while let Some(t) = submissions.join_next().await {
        tasks.push(t.unwrap());
    }
    assert!(tasks.iter().all(|t| t.id == tasks[0].id));
    assert_eq!(
        initial_facts.lists.load(Ordering::SeqCst),
        0,
        "admission must not build a snapshot"
    );
    let a = tasks[0].clone();
    let a2 = k
        .submit_task(p, Some(initial_facts.rows[1].id), None, None, actor())
        .await
        .unwrap();
    let b = k.submit_task(q, None, None, None, actor()).await.unwrap();
    let (one, two) = tokio::join!(k.claim_task(1), k.claim_task(1));
    let claims: Vec<_> = [one.unwrap(), two.unwrap()].into_iter().flatten().collect();
    assert_eq!(claims.len(), 1);
    let claimed = &claims[0];
    assert_eq!(claimed.id, a.id);
    let c = k.prepare_task(claimed).await.unwrap();
    assert_eq!(c.target, Some(e));
    assert!(
        k.claim_task(1).await.unwrap().is_none(),
        "busy workers retain queued submissions"
    );
    // The second scope cannot run alongside its project even when there is capacity.
    let other = k.claim_task(8).await.unwrap().unwrap();
    assert_eq!(other.id, b.id);
    assert!(k.claim_task(8).await.unwrap().is_none());
    k.stop_task(q, b.id).await.unwrap();
    let batch = k
        .open_candidate_batch(p, c.id, claimed.run_id, author())
        .await
        .unwrap();
    k.append_candidate(
        p,
        c.id,
        claimed.run_id,
        batch,
        &[Command::PutEndpointNote {
            endpoint: e,
            name: Some("saved batch".into()),
            purpose: None,
            context: Default::default(),
        }],
    )
    .await
    .unwrap();
    k.freeze_candidate_input(
        p,
        c.id,
        claimed.run_id.unwrap(),
        json!({"version":2,"fixture":"frozen"}),
    )
    .await
    .unwrap();
    let current = k.task(p, a.id).await.unwrap();
    expire(&store, &current).await;
    assert!(
        k.heartbeat_task(claimed).await.is_err(),
        "expired workers cannot revive themselves"
    );
    assert!(
        k.append_candidate(p, c.id, claimed.run_id, batch, &[])
            .await
            .is_err(),
        "lease expiration fences writes even before takeover"
    );
    let takeover = k.claim_task(1).await.unwrap().unwrap();
    assert_eq!(takeover.id, a.id);
    assert_eq!(takeover.generation, 2);
    let resumed = k.prepare_task(&takeover).await.unwrap();
    assert_eq!(resumed.id, c.id);
    assert_eq!(resumed.view.notes[0].name.as_deref(), Some("saved batch"));
    assert_eq!(
        resumed.execution_input,
        c.execution_input
            .or(Some(json!({"version":2,"fixture":"frozen"})))
    );
    for result in [
        k.heartbeat_candidate(p, c.id, claimed.run_id.unwrap())
            .await,
        k.freeze_candidate_input(p, c.id, claimed.run_id.unwrap(), json!({}))
            .await,
        k.checkpoint_candidate(
            p,
            c.id,
            CandidateState::Ready,
            json!({"run_id":claimed.run_id}),
        )
        .await,
        k.close_candidate_batch(p, c.id, claimed.run_id, batch, RoundState::Done)
            .await,
    ] {
        assert!(result.is_err());
    }
    assert!(k.finish_task(claimed, TaskState::Done, None).await.is_err());
    k.checkpoint_candidate(
        p,
        c.id,
        CandidateState::Partial,
        json!({"run_id":takeover.run_id}),
    )
    .await
    .unwrap();
    assert_eq!(
        k.task(p, a.id).await.unwrap().state,
        TaskState::Failed,
        "candidate and task finish atomically"
    );
    // A new project gets a turn before a continuously backlogged old project.
    let r = ProjectId::new();
    let r1 = k.submit_task(r, None, None, None, actor()).await.unwrap();
    let fair = k.claim_task(1).await.unwrap().unwrap();
    assert_eq!(fair.id, r1.id);
    k.stop_task(r, r1.id).await.unwrap();
    assert_eq!(k.claim_task(1).await.unwrap().unwrap().id, a2.id);
    k.stop_task(p, a2.id).await.unwrap();
    assert!(k.task(q, a.id).await.is_err());
    assert!(k.stop_task(q, a.id).await.is_err());
    // Explicit resume retains task identity; original candidate cancel also stops queued resume.
    let queued = k
        .submit_task(p, None, None, Some(c.id), actor())
        .await
        .unwrap();
    assert_eq!(queued.id, a.id);
    k.cancel_candidate(p, c.id).await.unwrap();
    assert_eq!(k.task(p, a.id).await.unwrap().state, TaskState::Stopped);
    assert!(k.claim_task(1).await.unwrap().is_none());
    k.submit_task(p, None, None, Some(c.id), actor())
        .await
        .unwrap();
    k.apply_candidate_as(p, c.id, actor()).await.unwrap();
    assert_eq!(k.task(p, a.id).await.unwrap().state, TaskState::Stopped);
    assert!(k.claim_task(1).await.unwrap().is_none());
    let t = k.submit_task(p, None, None, None, actor()).await.unwrap();
    let run = k.claim_task(1).await.unwrap().unwrap();
    assert_eq!(run.id, t.id);
    let c = k.prepare_task(&run).await.unwrap();
    k.stop_task(p, t.id).await.unwrap();
    assert!(
        k.open_candidate_batch(p, c.id, run.run_id, author())
            .await
            .is_err()
    );
    k.submit_task(p, None, None, Some(c.id), actor())
        .await
        .unwrap();
    k.discard_candidate_as(p, c.id, actor()).await.unwrap();
    assert_eq!(k.task(p, t.id).await.unwrap().state, TaskState::Stopped);
    // The same clear transaction removes stopped task identities; no task can resurrect it.
    let pool = sqlx::PgPool::connect(&fixture::database_url())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    store.prepare_project_reset(&mut tx, p).await.unwrap();
    store.reset_project_content(&mut tx, p).await.unwrap();
    tx.commit().await.unwrap();
    assert!(k.task(p, t.id).await.is_err());
    assert!(k.claim_task(1).await.unwrap().is_none());
    pool.close().await;
    // A queued request captures input only when claimed. A crash before freeze reuses its candidate.
    let fresh = facts();
    let project = fresh.rows[0].project_id;
    let earlier = Knowledge::new(store.clone(), fresh.clone());
    let queued = earlier
        .submit_task(project, None, None, None, actor())
        .await
        .unwrap();
    let mut latest = fresh.clone();
    let mut new_endpoint = latest.rows[0].clone();
    new_endpoint.id = EndpointId::new();
    latest.rows.push(new_endpoint.clone());
    let later = Knowledge::new(store.clone(), latest);
    let claim = later.claim_task(1).await.unwrap().unwrap();
    assert_eq!(claim.id, queued.id);
    let first = later.prepare_task(&claim).await.unwrap();
    assert!(first.base.endpoints.contains(&new_endpoint.id));
    expire(&store, &later.task(project, queued.id).await.unwrap()).await;
    let recovery = later.claim_task(1).await.unwrap().unwrap();
    let second = later.prepare_task(&recovery).await.unwrap();
    assert_eq!(second.id, first.id);
    later.stop_task(project, queued.id).await.unwrap();
    store.close().await;
    assert!(
        k.submit_task(p, None, None, None, actor()).await.is_err(),
        "closed database never reports accepted"
    );
}

#[path = "curation_queue_migration.rs"]
mod migration;

#[path = "curation_queue_children.rs"]
mod children;

#[path = "curation_queue_retry.rs"]
mod retry;

#[path = "curation_frozen_storage.rs"]
mod frozen;

#[path = "candidate_append.rs"]
mod append;

#[path = "history_retention.rs"]
mod retention;
