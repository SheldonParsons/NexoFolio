use async_trait::async_trait;
use nexofolio_application::OrganizationServices;
use nexofolio_contracts::{
    CatalogSnapshot, DirectoryCandidate, GeneratorInfo, KnowledgeEvent, KnowledgeEventKind,
    ProjectId, RebuildScope, Result,
};
use nexofolio_rebuild::DirectoryGenerator;
use nexofolio_triggers::{DirectoryMetrics, TriggerDecision, TriggerPolicy};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct QuietPolicy;
#[async_trait]
impl TriggerPolicy for QuietPolicy {
    async fn evaluate(&self, _: &KnowledgeEvent, _: &DirectoryMetrics) -> Result<TriggerDecision> {
        Ok(TriggerDecision::NoAction)
    }
}
struct InspectPolicy;
#[async_trait]
impl TriggerPolicy for InspectPolicy {
    async fn evaluate(&self, _: &KnowledgeEvent, _: &DirectoryMetrics) -> Result<TriggerDecision> {
        Ok(TriggerDecision::Inspect {
            scope: RebuildScope::Project,
            reasons: vec!["fixture".into()],
        })
    }
}
struct FixtureBuilder(Arc<AtomicUsize>);
#[async_trait]
impl DirectoryGenerator for FixtureBuilder {
    fn info(&self) -> Result<GeneratorInfo> {
        Ok(GeneratorInfo {
            adapter: "fixture".into(),
            model: "fixture".into(),
            prompt_version: "1".into(),
            prompt_sha256: "fixture".into(),
        })
    }
    async fn generate(&self, _: &CatalogSnapshot) -> Result<DirectoryCandidate> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(DirectoryCandidate {
            nodes: vec![],
            assignments: vec![],
            merge_groups: vec![],
        })
    }
}

#[tokio::test]
async fn trigger_replacement_does_not_invoke_or_change_builder() {
    let count = Arc::new(AtomicUsize::new(0));
    let event = KnowledgeEvent {
        project_id: ProjectId::new(),
        position: 1,
        kind: KnowledgeEventKind::InspectionRequested,
    };
    let first = OrganizationServices::new(
        Arc::new(QuietPolicy),
        Arc::new(FixtureBuilder(count.clone())),
    );
    let second = OrganizationServices::new(
        Arc::new(InspectPolicy),
        Arc::new(FixtureBuilder(count.clone())),
    );
    assert!(matches!(
        first
            .inspect(&event, &DirectoryMetrics::default())
            .await
            .unwrap(),
        TriggerDecision::NoAction
    ));
    assert!(matches!(
        second
            .inspect(&event, &DirectoryMetrics::default())
            .await
            .unwrap(),
        TriggerDecision::Inspect { .. }
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    second
        .build_candidate(&CatalogSnapshot {
            project_id: event.project_id,
            project_name: "fixture".into(),
            interfaces: vec![],
        })
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
