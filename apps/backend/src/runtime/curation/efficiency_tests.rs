//! Isolated fake-model checks. No model service or real project is invoked.
use super::tests::*;
use super::*;
use async_trait::async_trait;
use nexofolio_contracts::endpoint::{
    EndpointFacts, FieldFacts, FieldLocation, FieldPath, PathSegment, ValueType,
};
use nexofolio_curate::CompletionError;
use serde_json::Value;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

struct BatchModel {
    inner: Arc<Model>,
    active: AtomicUsize,
    peak: AtomicUsize,
    partial: AtomicBool,
    paths: Mutex<Vec<Vec<String>>>,
}
#[async_trait]
impl Completions for BatchModel {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        let Some(raw) = prompt.split("字段和可核查的证据：").nth(1) else {
            return self.inner.complete(prompt).await;
        };
        let fields: Vec<Value> = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        self.paths.lock().unwrap().push(
            fields
                .iter()
                .map(|f| f["path"].as_str().unwrap().into())
                .collect(),
        );
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let omit_last = self.partial.swap(false, Ordering::SeqCst);
        let count = fields.len() - usize::from(omit_last);
        Ok(serde_json::json!({"fields":fields.iter().take(count).map(|f| serde_json::json!({"key":f["key"],"display_name":"合成字段","description":"业务字段，具体含义待确认"})).collect::<Vec<_>>()}).to_string())
    }
}
async fn setup(
    count: usize,
    partial: bool,
) -> (
    Curation,
    ProjectId,
    Arc<Memory>,
    Arc<BatchModel>,
    EndpointFacts,
) {
    let (mut s, project, memory, inner) = fixture(false);
    let mut all = Vec::new();
    for summary in s.endpoints.list(project).await.unwrap() {
        let mut facts = s.endpoints.get(summary.id).await.unwrap().unwrap();
        facts.examples.clear();
        facts.fields = if facts.summary.path_template == "/api/1" {
            (0..count)
                .map(|i| FieldFacts {
                    location: FieldLocation::ResponseBody { status: 200 },
                    path: FieldPath(vec![PathSegment::Key(format!("f{i:03}"))]),
                    types: vec![ValueType::String],
                    labels: vec![],
                    differs_between_environments: false,
                    declared: None,
                    conflict: None,
                })
                .collect()
        } else {
            vec![]
        };
        all.push(facts);
    }
    let facts = all
        .iter()
        .find(|f| f.summary.path_template == "/api/1")
        .unwrap()
        .clone();
    s.endpoints = Arc::new(Endpoints(all));
    let model = Arc::new(BatchModel {
        inner,
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        partial: AtomicBool::new(partial),
        paths: Mutex::default(),
    });
    s.describe = Arc::new(Describe::new(
        model.clone() as Arc<dyn Completions>,
        "fixture".into(),
    ));
    (s, project, memory, model, facts)
}

#[tokio::test]
async fn one_large_endpoint_uses_the_pool_without_changing_field_semantics() {
    let (s, project, memory, model, facts) = setup(96, false).await;
    let s = s.with_concurrency(8, 4);
    let started = Instant::now();
    let progress = execute(&s, project).await;
    let parallel_ms = started.elapsed().as_millis();
    assert_eq!(progress.status, Status::Done);
    assert_eq!((progress.fields_updated, progress.fields_failed), (96, 0));
    assert_eq!(model.peak.load(Ordering::SeqCst), s.field_concurrency);
    let knowledge = s.reader.endpoint(facts.summary.id).await.unwrap();
    let prepared = super::preparation::prepare(&s, &s.endpoints.list(project).await.unwrap())
        .await
        .unwrap();
    let inputs = prepared.inputs.get(&facts.summary.id).unwrap().clone();
    let started = Instant::now();
    let expected = s
        .describe
        .fields(&facts, &knowledge, &inputs)
        .await
        .unwrap();
    let sequential_ms = started.elapsed().as_millis();
    for command in expected {
        let nexofolio_contracts::knowledge::Command::PutFieldNote {
            path,
            text,
            values,
            context,
            ..
        } = command
        else {
            panic!()
        };
        let stored = knowledge.fields.iter().find(|f| f.path == path).unwrap();
        assert_eq!(
            (&stored.text, &stored.values, &stored.context),
            (&text, &values, &context)
        );
    }
    assert_eq!(memory.data.lock().unwrap().fields.len(), 96);
    assert!(progress.active.is_empty());
    assert_eq!(progress.completed_batches, progress.total_batches);
    assert!(progress.directory_completed);
    println!(
        "isolated fake-model batch benchmark: sequential={sequential_ms}ms parallel={parallel_ms}ms peak={} fields=96",
        model.peak.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn incomplete_model_reply_retries_only_the_missing_field_and_reuses_results_next_run() {
    let (s, project, memory, model, _) = setup(24, true).await;
    let progress = execute(&s, project).await;
    assert_eq!(progress.status, Status::Done);
    assert_eq!(progress.fields_updated, 24);
    assert_eq!(memory.data.lock().unwrap().fields.len(), 24);
    let sizes: Vec<_> = model.paths.lock().unwrap().iter().map(Vec::len).collect();
    let mut sizes = sizes;
    sizes.sort();
    assert_eq!(sizes, [1, 12, 12]);
    assert!(
        progress
            .activity
            .iter()
            .any(|e| e.kind == "retry" && e.fields == Some(1))
    );
    model.paths.lock().unwrap().clear();
    let repeat = execute(&s, project).await;
    assert_eq!(repeat.fields_reused, 24);
    assert_eq!(repeat.fields_updated, 0);
    assert!(model.paths.lock().unwrap().is_empty());
}
