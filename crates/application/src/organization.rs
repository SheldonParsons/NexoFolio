//! Trigger inspection returns an intent only. New reconstruction starts via MaintenanceStore.
use nexofolio_contracts::{KnowledgeEvent, Result};
use nexofolio_triggers::{DirectoryMetrics, TriggerDecision, TriggerPolicy};
use std::sync::Arc;
pub struct OrganizationServices {
    trigger: Arc<dyn TriggerPolicy>,
}
impl OrganizationServices {
    pub fn new(trigger: Arc<dyn TriggerPolicy>) -> Self {
        Self { trigger }
    }
    pub async fn inspect(
        &self,
        event: &KnowledgeEvent,
        metrics: &DirectoryMetrics,
    ) -> Result<TriggerDecision> {
        self.trigger.evaluate(event, metrics).await
    }
}
