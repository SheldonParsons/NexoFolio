//! Shared, versioned contract between knowledge storage and directory generation.
use crate::{
    CatalogVersion, DirectoryId, EnvironmentId, InterfaceId, JobId, ProjectId, RevisionId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CATALOG_PREVIEW_VERSION: &str = "catalog-preview-1";
pub const MAX_PREVIEW_INTERFACES: usize = 200;
pub const MAX_PREVIEW_BYTES: usize = 1_048_576;
pub const MAX_CANDIDATE_BYTES: usize = 524_288;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    pub project_id: ProjectId,
    pub project_name: String,
    pub interfaces: Vec<CatalogInterface>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogInterface {
    pub interface_id: InterfaceId,
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_path: Option<crate::PathIdentity>,
    pub environments: Vec<CatalogEnvironment>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogEnvironment {
    pub environment_id: EnvironmentId,
    pub environment_name: String,
    pub revision_id: RevisionId,
    pub definition: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirectoryCandidate {
    pub nodes: Vec<PreviewNode>,
    pub assignments: Vec<PreviewAssignment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_groups: Vec<InterfaceMergeGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceMergeGroup {
    pub representative_id: InterfaceId,
    pub member_ids: Vec<InterfaceId>,
    pub path_template: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewNode {
    pub id: DirectoryId,
    pub parent: Option<DirectoryId>,
    pub name: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewAssignment {
    pub interface_id: InterfaceId,
    /// null explicitly leaves this interface unclassified.
    pub directory_id: Option<DirectoryId>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewIssue {
    pub code: String,
    pub subject: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DirectoryCounts {
    pub directory_id: DirectoryId,
    pub direct_interfaces: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewMetrics {
    pub snapshot_interfaces: usize,
    pub assigned_interfaces: usize,
    pub unclassified_interfaces: usize,
    pub unclassified_ratio: f64,
    pub directory_count: usize,
    pub max_depth: usize,
    pub directories: Vec<DirectoryCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_interfaces: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewReview {
    /// Structural validity only; never a claim of semantic quality or publication approval.
    pub structurally_valid: bool,
    pub issues: Vec<PreviewIssue>,
    pub warnings: Vec<PreviewIssue>,
    pub metrics: PreviewMetrics,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct GeneratorInfo {
    pub adapter: String,
    pub model: String,
    pub prompt_version: String,
    pub prompt_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    Pending,
    Running,
    Ready,
    Rejected,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewTask {
    pub task_id: JobId,
    pub candidate_id: CatalogVersion,
    pub contract_version: String,
    pub status: PreviewStatus,
    pub snapshot_at: String,
    pub snapshot_sha256: String,
    pub snapshot: CatalogSnapshot,
    pub generation: i64,
    pub generator: Option<GeneratorInfo>,
    pub error_code: Option<String>,
    pub candidate: Option<DirectoryCandidate>,
    pub review: Option<PreviewReview>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_contract_schemas_match_rust() {
        for (actual, bundled) in [
            (
                serde_json::to_value(schemars::schema_for!(DirectoryCandidate)).unwrap(),
                include_str!("../../../contracts/catalog-preview/candidate.schema.json"),
            ),
            (
                serde_json::to_value(schemars::schema_for!(CatalogSnapshot)).unwrap(),
                include_str!("../../../contracts/catalog-preview/snapshot.schema.json"),
            ),
            (
                serde_json::to_value(schemars::schema_for!(PreviewTask)).unwrap(),
                include_str!("../../../contracts/catalog-preview/task.schema.json"),
            ),
            (
                serde_json::to_value(schemars::schema_for!(CatalogPreviewPage)).unwrap(),
                include_str!("../../../contracts/catalog-preview/page.schema.json"),
            ),
            (
                serde_json::to_value(schemars::schema_for!(CatalogPreviewDetail)).unwrap(),
                include_str!("../../../contracts/catalog-preview/detail.schema.json"),
            ),
        ] {
            assert_eq!(actual, serde_json::from_str::<Value>(bundled).unwrap());
        }
        assert!(
            serde_json::from_value::<DirectoryCandidate>(
                serde_json::json!({"nodes":[],"assignments":[],"publish":true})
            )
            .is_err()
        );
    }
}

/// Public read-only HTTP views. Administrative create/run methods are not exposed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogPreviewListItem {
    pub task_id: JobId,
    pub candidate_id: CatalogVersion,
    pub status: PreviewStatus,
    pub snapshot_at: String,
    pub interface_count: usize,
    pub directory_count: Option<usize>,
    pub unclassified_count: Option<usize>,
    pub structurally_valid: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogPreviewPage {
    pub items: Vec<CatalogPreviewListItem>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogPreviewDetail {
    /// True only when this candidate is the currently active official version.
    pub published: bool,
    pub task: PreviewTask,
}
