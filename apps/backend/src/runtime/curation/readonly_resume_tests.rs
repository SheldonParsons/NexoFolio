//! Opt-in diagnostic: SELECT only, no migrations, candidates, model calls or business HTTP writes.
use super::*;
use async_trait::async_trait;
use nexofolio_common::Secret;
use nexofolio_curate::CompletionError;
use nexofolio_knowledge::Knowledge;
use nexofolio_knowledge_adapter::PostgresKnowledge;
use nexofolio_observe::Observe;
use nexofolio_observe_adapter::PostgresObserve;
use std::time::Duration;
struct NeverCall;
#[async_trait]
impl Completions for NeverCall {
    async fn complete(&self, _: &str) -> Result<String, CompletionError> {
        panic!("read-only validation cannot call any model")
    }
}
#[tokio::test]
#[ignore = "requires explicit READONLY_DATABASE_URL, READONLY_PROJECT_ID and READONLY_CANDIDATE_ID; SELECT only"]
async fn retained_legacy_candidate_input_can_be_verified_without_starting_it() {
    let url =
        Secret::new(std::env::var("READONLY_DATABASE_URL").expect("read-only diagnostic database"));
    let project = std::env::var("READONLY_PROJECT_ID")
        .unwrap()
        .parse()
        .unwrap();
    let id = std::env::var("READONLY_CANDIDATE_ID")
        .unwrap()
        .parse()
        .unwrap();
    let store = PostgresObserve::new(&url, 2, Duration::from_secs(10)).unwrap();
    let k = Arc::new(Knowledge::new(
        PostgresKnowledge::new(&url, 2, Duration::from_secs(10)).unwrap(),
        Observe::new(store.clone()),
    ));
    let model: Arc<dyn Completions> = Arc::new(NeverCall);
    let s = Curation::new(
        k.clone(),
        k.clone(),
        Arc::new(Observe::new(store)),
        Arc::new(Describe::new(model.clone(), "never-called".into())),
        Arc::new(Organize::new(model, "never-called".into())),
    )
    .with_candidates(k.clone());
    let candidate = k.candidate(project, id).await.unwrap();
    let frozen = super::recovery::legacy_input(&s, &candidate).await;
    assert!(
        frozen.is_ok(),
        "legacy input compatibility: {}",
        frozen.err().map(|e| e.to_string()).unwrap_or_default()
    );
    let after = k.candidate(project, id).await.unwrap();
    assert_eq!(candidate.updated_at, after.updated_at);
    assert_eq!(candidate.state, after.state);
    assert!(candidate.execution_input.is_none() && after.execution_input.is_none());
    println!("read-only legacy compatibility verified; candidate unchanged; model calls=0");
}
