use nexofolio_contracts::{CatalogSnapshot, DirectoryCandidate, KnowledgeEvent, Result};
use nexofolio_rebuild::DirectoryGenerator;
use nexofolio_triggers::{DirectoryMetrics, TriggerDecision, TriggerPolicy};
use std::sync::Arc;

/// Wiring boundary only: inspecting a trigger cannot itself launch a rebuild.
pub struct OrganizationServices {
    trigger: Arc<dyn TriggerPolicy>,
    builder: Arc<dyn DirectoryGenerator>,
}

impl OrganizationServices {
    pub fn new(trigger: Arc<dyn TriggerPolicy>, builder: Arc<dyn DirectoryGenerator>) -> Self {
        Self { trigger, builder }
    }
    pub async fn inspect(
        &self,
        event: &KnowledgeEvent,
        metrics: &DirectoryMetrics,
    ) -> Result<TriggerDecision> {
        self.trigger.evaluate(event, metrics).await
    }
    pub async fn build_candidate(&self, snapshot: &CatalogSnapshot) -> Result<DirectoryCandidate> {
        self.builder.generate(snapshot).await
    }
}
