//! Reconstruction inputs and candidates. No persistence or publication permission.
use async_trait::async_trait;
use nexofolio_contracts::{
    CatalogVersion, InterfaceId, ProjectId, RebuildScope, Result, RevisionId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotEntry {
    pub interface_id: InterfaceId,
    pub revision: RevisionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RebuildRequest {
    pub project_id: ProjectId,
    pub scope: RebuildScope,
    pub base_catalog: CatalogVersion,
    pub event_position: u64,
    pub inputs: Vec<SnapshotEntry>,
    pub policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CandidatePlan {
    pub request: RebuildRequest,
    pub candidate: CatalogVersion,
    pub changes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CandidateEvaluation {
    pub candidate: CatalogVersion,
    pub acceptable: bool,
    pub reasons: Vec<String>,
}

#[async_trait]
pub trait CatalogBuilder: Send + Sync {
    async fn build(&self, request: RebuildRequest) -> Result<CandidatePlan>;
}

#[async_trait]
pub trait CatalogEvaluator: Send + Sync {
    async fn evaluate(&self, candidate: &CandidatePlan) -> Result<CandidateEvaluation>;
}
