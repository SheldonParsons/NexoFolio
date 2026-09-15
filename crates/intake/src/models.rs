//! Legacy internal skeleton contracts; external uploads use batch.rs and the versioned machine schemas.
use nexofolio_contracts::{InterfaceId, ProjectId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureInput {
    pub project_id: ProjectId,
    pub capture_key: String,
    pub method: String,
    pub url: String,
    pub request: Value,
    pub response: Value,
    pub source_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceIdentity {
    pub project_id: ProjectId,
    pub service: String,
    pub method: String,
    pub normalized_path: String,
    pub version_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StructuralFingerprint {
    pub algorithm_version: String,
    pub digest: String,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ValidationResult {
    Valid,
    Rejected { reason: String },
    Incomplete { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeduplicationResult {
    Known { interface_id: InterfaceId },
    RequiresAnalysis,
    AlreadyQueued { receipt_id: String },
}
