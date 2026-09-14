//! Trigger decisions only: this package cannot execute reconstruction.
use async_trait::async_trait;
use nexofolio_contracts::{KnowledgeEvent, RebuildScope, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DirectoryMetrics {
    pub interface_count: u64,
    pub unresolved_count: u64,
    pub observed_retrieval_failures: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TriggerDecision {
    NoAction,
    Inspect {
        scope: RebuildScope,
        reasons: Vec<String>,
    },
    RequestRebuild {
        scope: RebuildScope,
        reasons: Vec<String>,
        policy_version: String,
    },
}

#[async_trait]
pub trait TriggerPolicy: Send + Sync {
    async fn evaluate(
        &self,
        event: &KnowledgeEvent,
        metrics: &DirectoryMetrics,
    ) -> Result<TriggerDecision>;
}
