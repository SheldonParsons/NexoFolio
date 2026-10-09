//! A name-only retry must follow the partial save ACK and must not count the field twice.
use super::*;
use async_trait::async_trait;
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::*, knowledge::*, observation::*, testing::sample_exchange};
use nexofolio_curate::CompletionError;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_contracts::testing::InMemoryKnowledge;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Clone)]
struct Observed {
    facts: EndpointFacts,
    raw: Value,
}
#[async_trait]
impl EndpointReader for Observed {
    async fn list(&self, p: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError> {
        Ok(if p == self.facts.summary.project_id {
            vec![self.facts.summary.clone()]
        } else {
            vec![]
        })
    }
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError> {
        Ok((id == self.facts.summary.id).then(|| self.facts.clone()))
    }
    async fn example(&self, id: EndpointId, _: &str) -> Result<Option<Value>, EndpointError> {
        Ok((id == self.facts.summary.id).then(|| self.raw.clone()))
    }
}
struct Model {
    field_calls: AtomicUsize,
    knowledge: Arc<dyn KnowledgeCandidates>,
    project: ProjectId,
}
#[async_trait]
impl Completions for Model {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        if !prompt.contains("字段和可核查的证据：") {
            return Ok(json!({"name":"合成接口","purpose":"查询合成记录"}).to_string());
        }
        if self.field_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(json!({"fields":[{"key":"0","description":"记录取值"}]}).to_string());
        }
        assert!(prompt.contains("\"requested\":[\"display_name\"]"));
        let saved = self.knowledge.candidates(self.project, 1).await.unwrap();
        assert_eq!(saved[0].progress["fields_pending"], 1);
        assert_eq!(saved[0].progress["fields_updated"], 1);
        let c = self
            .knowledge
            .candidate(self.project, saved[0].id)
            .await
            .unwrap();
        assert_eq!(c.view.fields[0].text, "记录取值");
        assert!(c.view.fields[0].context.display_name.is_none());
        Ok(json!({"fields":[{"key":"0","display_name":"取值","description":"不应覆盖已保存的说明"}]}).to_string())
    }
}
#[tokio::test]
async fn name_gap_is_saved_then_retried_without_duplicate_count_or_description_overwrite() {
    let (template, project, _, _) = super::tests::fixture(false);
    let mut facts = template
        .endpoints
        .get(template.endpoints.list(project).await.unwrap()[0].id)
        .await
        .unwrap()
        .unwrap();
    let endpoint = facts.summary.id;
    facts.fields = vec![FieldFacts {
        location: FieldLocation::RequestBody,
        path: FieldPath(vec![PathSegment::Key("value".into())]),
        types: vec![ValueType::Number],
        labels: vec![],
        declared: None,
        conflict: None,
        differs_between_environments: false,
    }];
    let mut raw = sample_exchange(project, EnvironmentId::new());
    if let Fact::Exchange(e) = &mut raw.fact {
        e.request.body = Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("{\"value\":0}".into()),
        };
    }
    facts.examples = vec![ExampleSummary {
        id: "fixture".into(),
        structure_id: "fixture".into(),
        retained_samples: 1,
        environment_id: raw.environment_id.unwrap(),
        address: ServiceAddress::parse("https://fixture.test").unwrap(),
        status: Some(200),
        calls: 1,
        first_seen: raw.observed_at,
        last_seen: raw.observed_at,
    }];
    let endpoints = Observed {
        facts,
        raw: serde_json::to_value(raw).unwrap(),
    };
    let knowledge = Arc::new(Knowledge::new(InMemoryKnowledge::new(), endpoints.clone()));
    let model = Arc::new(Model {
        field_calls: AtomicUsize::new(0),
        knowledge: knowledge.clone(),
        project,
    });
    let completions: Arc<dyn Completions> = model.clone();
    let s = Curation::new(
        knowledge.clone(),
        knowledge.clone(),
        Arc::new(endpoints),
        Arc::new(Describe::new(completions.clone(), "fixture".into())),
        Arc::new(Organize::new(completions, "fixture".into())),
    )
    .with_candidates(knowledge.clone());
    let start = s.start_target(project, Some(endpoint), None).await.unwrap();
    let id = start.candidate_id.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while knowledge.candidate(project, id).await.unwrap().state == CandidateState::Running {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let progress = s.progress(project);
    assert_eq!(progress.status, Status::Done);
    assert_eq!(progress.fields_updated, 1);
    assert_eq!(progress.fields_total, 1);
    assert_eq!(progress.fields_filled, 1);
    assert_eq!(progress.fields_pending, 0);
    assert_eq!(progress.fields_confirmation, 0);
    assert_eq!(model.field_calls.load(Ordering::SeqCst), 2);
    let saved = knowledge.candidate(project, id).await.unwrap();
    assert_eq!(saved.view.fields.len(), 1);
    let note = &saved.view.fields[0];
    assert_eq!(note.text, "记录取值");
    assert_eq!(note.context.display_name.as_deref(), Some("取值"));
    assert_eq!(
        note.context.observations.as_ref().unwrap().archive_id,
        Some(id)
    );
    let q = nexofolio_contracts::field_values::FieldValueQuery {
        project,
        archive: id,
        endpoint,
        field: note.path.clone(),
        offset: 0,
        limit: 12,
        environment: None,
    };
    let values = knowledge.observed_values(q).await.unwrap();
    assert_eq!(values.items[0].preview, "0");
    assert!(
        knowledge
            .endpoint(endpoint)
            .await
            .unwrap()
            .fields
            .is_empty()
    );
}
