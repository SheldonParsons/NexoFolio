use nexofolio_contracts::{
    CatalogVersion, ChangeId, DirectoryId, InterfaceId, ProjectId, RevisionId, SourceId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeState {
    Observed,
    Draft,
    Confirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceRecord {
    pub id: InterfaceId,
    pub project_id: ProjectId,
    pub revision: RevisionId,
    pub state: KnowledgeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceRevision {
    pub id: RevisionId,
    pub interface_id: InterfaceId,
    pub schema_version: u32,
    pub definition: Value,
    pub source_ids: Vec<SourceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub source_id: SourceId,
    pub interface_id: InterfaceId,
    pub revision: RevisionId,
    pub content: Value,
    pub source_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Applied,
    Unchanged,
    PendingAdjudication,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeProposal {
    pub id: ChangeId,
    pub interface_id: InterfaceId,
    pub base_revision: RevisionId,
    pub proposed_definition: Value,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogNode {
    pub id: DirectoryId,
    pub parent: Option<DirectoryId>,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct PublicationCondition {
    pub project_id: ProjectId,
    pub expected_current: CatalogVersion,
    pub candidate: CatalogVersion,
    pub covered_event_position: u64,
    pub execution_generation: u64,
}
