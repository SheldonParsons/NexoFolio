use async_trait::async_trait;
use nexofolio_application::OrganizationServices;
use nexofolio_contracts::{KnowledgeEvent, KnowledgeEventKind, ProjectId, RebuildScope, Result};
use nexofolio_triggers::{DirectoryMetrics, TriggerDecision, TriggerPolicy};
use std::sync::Arc;

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
#[tokio::test]
async fn trigger_replacement_returns_intents_without_a_generation_capability() {
    let event = KnowledgeEvent {
        project_id: ProjectId::new(),
        position: 1,
        kind: KnowledgeEventKind::InspectionRequested,
    };
    let first = OrganizationServices::new(Arc::new(QuietPolicy));
    let second = OrganizationServices::new(Arc::new(InspectPolicy));
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
}
