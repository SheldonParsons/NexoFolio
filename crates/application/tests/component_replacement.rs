use async_trait::async_trait;
use nexofolio_application::OrganizationServices;
use nexofolio_contracts::{
    CatalogVersion, KnowledgeEvent, KnowledgeEventKind, ProjectId, RebuildScope, Result,
};
use nexofolio_rebuild::{CandidatePlan, CatalogBuilder, RebuildRequest};
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
impl CatalogBuilder for FixtureBuilder {
    async fn build(&self, request: RebuildRequest) -> Result<CandidatePlan> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(CandidatePlan {
            request,
            candidate: CatalogVersion::new(),
            changes: serde_json::json!([]),
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
        .build_candidate(RebuildRequest {
            project_id: event.project_id,
            scope: RebuildScope::Project,
            base_catalog: CatalogVersion::new(),
            event_position: 1,
            inputs: vec![],
            policy_version: "fixture".into(),
        })
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
