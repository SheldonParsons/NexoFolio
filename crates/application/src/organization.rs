use nexofolio_contracts::{KnowledgeEvent, Result};
use nexofolio_rebuild::{CandidatePlan, CatalogBuilder, RebuildRequest};
use nexofolio_triggers::{DirectoryMetrics, TriggerDecision, TriggerPolicy};
use std::sync::Arc;

/// Wiring boundary only: inspecting a trigger cannot itself launch a rebuild.
pub struct OrganizationServices {
    trigger: Arc<dyn TriggerPolicy>,
    builder: Arc<dyn CatalogBuilder>,
}

impl OrganizationServices {
    pub fn new(trigger: Arc<dyn TriggerPolicy>, builder: Arc<dyn CatalogBuilder>) -> Self {
        Self { trigger, builder }
    }
    pub async fn inspect(
        &self,
        event: &KnowledgeEvent,
        metrics: &DirectoryMetrics,
    ) -> Result<TriggerDecision> {
        self.trigger.evaluate(event, metrics).await
    }
    pub async fn build_candidate(&self, request: RebuildRequest) -> Result<CandidatePlan> {
        self.builder.build(request).await
    }
}
