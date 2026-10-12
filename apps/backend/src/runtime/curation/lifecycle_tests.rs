//! The real background pipeline with synthetic inputs and a model that must not run.
use super::*;
use async_trait::async_trait;
use nexofolio_contracts::{endpoint::*, knowledge::*, observation::*, testing::sample_exchange};
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
struct NoModel;
#[async_trait]
impl Completions for NoModel {
    async fn complete(&self, _: &str) -> Result<String, nexofolio_curate::CompletionError> {
        panic!("empty effective project must not call model")
    }
}
#[tokio::test]
async fn all_retired_interfaces_produce_reviewable_empty_knowledge_without_model_calls() {
    let (template, project, _, _) = tests::fixture(false);
    let summaries = template.endpoints.list(project).await.unwrap();
    let mut facts = Vec::new();
    for e in &summaries {
        facts.push(template.endpoints.get(e.id).await.unwrap().unwrap());
    }
    let k = Arc::new(Knowledge::new(
        InMemoryKnowledge::new(),
        tests::Endpoints(facts.clone()),
    ));
    let actor = CandidateActor::User {
        id: nexofolio_common::UserId::new(),
        display_name: Some("fixture".into()),
    };
    for (version, e) in summaries.iter().enumerate() {
        k.change_lifecycle(
            project,
            LifecycleChange {
                target: LifecycleTarget::Endpoint { id: e.id },
                retired: true,
                reason: "合成废弃".into(),
                expected_version: version as u64,
            },
            actor.clone(),
        )
        .await
        .unwrap();
    }
    let model: Arc<dyn Completions> = Arc::new(NoModel);
    let s = Curation::new(
        k.clone(),
        k.clone(),
        Arc::new(tests::Endpoints(facts)),
        Arc::new(Describe::new(model.clone(), "fixture".into())),
        Arc::new(Organize::new(model, "fixture".into())),
    )
    .with_candidates(k.clone());
    let accepted = s.start(project).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while k
            .candidate(project, accepted.candidate_id.unwrap())
            .await
            .unwrap()
            .state
            == CandidateState::Running
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let saved = k
        .candidate(project, accepted.candidate_id.unwrap())
        .await
        .unwrap();
    assert_eq!(saved.state, CandidateState::Ready);
    assert!(saved.view.endpoints.is_empty());
    assert!(saved.view.fields.is_empty());
    assert!(saved.view.links.is_empty());
    assert!(saved.view.folders.is_empty());
    assert_eq!(
        saved.execution_input.as_ref().unwrap()["lifecycle_version"],
        2
    );
    k.apply_candidate(project, saved.id, actor.user_id().unwrap())
        .await
        .unwrap();
    assert!(!k.lifecycle(project).await.unwrap().pending());
}
#[test]
fn field_values_and_ancestor_objects_exclude_retired_children_but_raw_samples_remain_intact() {
    let project = ProjectId::new();
    let endpoint = EndpointId::new();
    let mut observation = sample_exchange(project, nexofolio_common::EnvironmentId::new());
    let Fact::Exchange(e) = &mut observation.fact else {
        panic!()
    };
    e.request.body = Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(
            r#"{"data":{"old":"retired-secret","kept":"valid"},"database":"other"}"#.into(),
        ),
    };
    let raw = serde_json::to_value(&observation).unwrap();
    let location = FieldLocation::RequestBody;
    let path = FieldPath(vec![
        PathSegment::Key("data".into()),
        PathSegment::Key("old".into()),
    ]);
    let state = DocumentLifecycle {
        version: 1,
        reconciled_version: 0,
        aliases: vec![],
        decisions: vec![Retirement {
            target: LifecycleTarget::Field {
                endpoint,
                location,
                path: path.clone(),
            },
            reason: "fixture".into(),
            actor: CandidateActor::Unknown,
            at: observation.observed_at,
            version: 1,
            retired: true,
            folders: vec![],
        }],
    };
    let active = nexofolio_curate::active_observation(raw.clone(), &state, endpoint, "/fixture");
    assert!(!active.to_string().contains("retired-secret"));
    assert!(raw.to_string().contains("retired-secret"));
    assert!(active.to_string().contains("valid"));
    assert!(active.to_string().contains("database"));
    let field = |path: FieldPath| FieldFacts {
        location,
        path,
        types: vec![ValueType::Object],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    };
    let mut facts = EndpointFacts {
        summary: EndpointSummary {
            id: endpoint,
            project_id: project,
            method: "POST".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        fields: vec![field(FieldPath::default()), field(path)],
        examples: vec![],
        requests: vec![],
    };
    nexofolio_curate::active_facts(&mut facts, &state);
    assert_eq!(facts.fields.len(), 1);
    let archive = nexofolio_observe::archive_values(&facts, &[("fixture".into(), Some(active))]);
    assert!(
        !serde_json::to_string(&archive)
            .unwrap()
            .contains("retired-secret")
    );
    assert_eq!(archive.fields.len(), 1);
    assert!(
        archive
            .documents
            .values()
            .all(|d| d.state == "partial" && d.source.complete == Some(false)),
        "projected containers must not claim complete original evidence"
    );
}

#[tokio::test]
async fn preparation_refuses_an_identity_merged_after_the_inventory_read() {
    struct Changed {
        original: Arc<dyn EndpointReader>,
        into: EndpointId,
    }
    #[async_trait]
    impl EndpointReader for Changed {
        async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
            self.original.list(p).await
        }
        async fn get(&self, _: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
            self.original.get(self.into).await
        }
        async fn example(
            &self,
            _: EndpointId,
            _: &str,
        ) -> Result<Option<serde_json::Value>, EndpointError> {
            panic!("must not freeze another identity's samples")
        }
    }
    let (mut s, p, _, _) = tests::fixture(false);
    let all = s.endpoints.list(p).await.unwrap();
    s.endpoints = Arc::new(Changed {
        original: s.endpoints.clone(),
        into: all[1].id,
    });
    assert!(matches!(
        super::snapshot::evidence_snapshot(&s, &all[..1], None).await,
        Err(CurationError::Conflict(_))
    ));
}
