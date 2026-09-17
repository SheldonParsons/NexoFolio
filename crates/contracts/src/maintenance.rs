//! Revision-bound knowledge maintenance. Evidence is source material, not a model instruction.
use crate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Observed,
    Inferred,
    NeedsReview,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticTarget {
    Interface { interface_id: InterfaceId },
    Field { field: FieldRef },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnumEntry {
    pub state: UiValueState,
    pub value: Value,
    pub label: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticValue {
    Description {
        text: String,
    },
    ParameterRelation {
        source: FieldRef,
        target: FieldRef,
        transform: String,
        conditions: Value,
    },
    Enum {
        entries: Vec<EnumEntry>,
        complete: bool,
        scope: Value,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeEvidenceRef {
    #[schemars(with = "KnowledgeReferenceKind")]
    pub kind: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnotationDraft {
    pub id: Uuid,
    pub target: SemanticTarget,
    pub value: SemanticValue,
    pub evidence: Vec<KnowledgeEvidenceRef>,
    pub verification: Verification,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DefinitionBasis {
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    pub revision_id: RevisionId,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SemanticAnnotation {
    pub annotation: AnnotationDraft,
    pub basis: Vec<DefinitionBasis>,
    pub source_run_id: Uuid,
    pub stale: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeField {
    pub id: String,
    pub reference: EvidenceFieldRef,
    pub schema: Value,
    pub schema_pointers: Vec<String>,
    pub ancestors: Vec<String>,
    pub existing_annotations: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeSnapshot {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub base_generation: i64,
    pub base_catalog_version: Option<CatalogVersion>,
    pub base_knowledge_version: Option<Uuid>,
    pub system_directory_id: DirectoryId,
    pub catalog: DirectoryCandidate,
    pub interfaces: Vec<CatalogInterface>,
    pub fields: Vec<KnowledgeField>,
    pub annotations: Vec<SemanticAnnotation>,
    pub facts: Vec<EvidenceFact>,
    pub pending_observations: i64,
    pub directory_metrics: Option<PreviewMetrics>,
    /// Last published maintenance result, informational only; never independent evidence.
    pub previous_maintenance: Option<serde_json::Value>,
    /// None identifies a historical snapshot without the frozen-input contract.
    #[serde(default)]
    pub inputs: Option<SnapshotInputs>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceStrategy {
    Keep,
    Insert,
    Partial,
    Full,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceAction {
    SetDirectory {
        node: PreviewNode,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    RemoveDirectory {
        directory_id: DirectoryId,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    AssignInterface {
        interface_id: InterfaceId,
        directory_id: Option<DirectoryId>,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    ReplaceCatalog {
        candidate: DirectoryCandidate,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    UpsertMergeGroup {
        group: InterfaceMergeGroup,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    RemoveMergeGroup {
        representative_id: InterfaceId,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
    UpsertAnnotation {
        annotation: Box<AnnotationDraft>,
        reason: String,
    },
    RetractAnnotation {
        annotation_id: Uuid,
        reason: String,
        evidence: Vec<KnowledgeEvidenceRef>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaintenancePlan {
    pub strategy: MaintenanceStrategy,
    pub reason: String,
    pub expected_benefit: String,
    pub actions: Vec<MaintenanceAction>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldAssessment {
    pub unit_id: String,
    pub field_id: String,
    #[schemars(with = "FieldReviewDisposition")]
    pub disposition: String,
    pub note: String,
    pub evidence: Vec<KnowledgeEvidenceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SegmentReview {
    pub segment_id: String,
    pub assessments: Vec<FieldAssessment>,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceRead {
    pub kind: String,
    pub ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceReply {
    Read {
        requests: Vec<MaintenanceRead>,
    },
    Review {
        review: SegmentReview,
    },
    Plan {
        plan: MaintenancePlan,
    },
    Summary {
        summary: String,
    },
    Image {
        read: bool,
        summary: String,
        limitations: Vec<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewCoverage {
    pub total_interfaces: usize,
    pub total_fields: usize,
    pub reviewed_fields: usize,
    pub total_segments: usize,
    pub completed_segments: usize,
    pub complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceCandidate {
    pub plan: MaintenancePlan,
    pub catalog: DirectoryCandidate,
    pub annotations: Vec<SemanticAnnotation>,
    pub review: PreviewReview,
    pub coverage: ReviewCoverage,
    pub issues: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceStatus {
    Pending,
    Running,
    Ready,
    Incomplete,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceRun {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub created_at: String,
    pub status: MaintenanceStatus,
    pub phase: String,
    pub snapshot_id: Uuid,
    pub base_generation: i64,
    pub coverage: ReviewCoverage,
    pub model_calls: u32,
    pub read_count: u32,
    pub error_code: Option<String>,
    pub candidate: Option<MaintenanceCandidate>,
    pub currently_published: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenancePage {
    pub items: Vec<MaintenanceSummary>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceSummary {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub created_at: String,
    pub status: MaintenanceStatus,
    pub phase: String,
    pub snapshot_id: Uuid,
    pub base_generation: i64,
    pub coverage: ReviewCoverage,
    pub model_calls: u32,
    pub read_count: u32,
    pub error_code: Option<String>,
    pub candidate_available: bool,
    pub currently_published: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartMaintenance {
    pub request_id: Uuid,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishKnowledge {
    pub request_id: Uuid,
    pub expected_generation: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreKnowledge {
    #[serde(deserialize_with = "required_knowledge_version")]
    #[schemars(required, schema_with = "nullable_knowledge_version_schema")]
    pub version_id: Option<Uuid>,
    pub request_id: Uuid,
    pub expected_generation: i64,
}
fn required_knowledge_version<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Uuid>, D::Error> {
    Deserialize::deserialize(d)
}
fn nullable_knowledge_version_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::schema_for!(Option<Uuid>)
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeActivation {
    pub request_id: Uuid,
    pub generation: i64,
    pub version_id: Option<Uuid>,
    pub catalog_version_id: Option<CatalogVersion>,
    pub changed: bool,
    pub replayed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeVersion {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub catalog_version_id: Option<CatalogVersion>,
    pub source_run_id: Option<Uuid>,
    pub created_at: String,
    pub current: bool,
    pub origin: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeVersionPage {
    pub items: Vec<KnowledgeVersion>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
    pub generation: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceKnowledge {
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    pub revision_id: RevisionId,
    pub generation: i64,
    pub knowledge_version_id: Option<Uuid>,
    pub annotations: Vec<SemanticAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceCheckpoint {
    pub id: String,
    pub phase: String,
    pub references: Vec<KnowledgeEvidenceRef>,
    pub review: Option<SegmentReview>,
    pub summary: Option<String>,
    pub data: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceCheckpointPage {
    pub items: Vec<MaintenanceCheckpoint>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldReviewDisposition {
    Keep,
    Change,
    Conflict,
    NeedsEvidence,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeReferenceKind {
    Field,
    Fact,
    Interface,
    Directory,
    Image,
    Summary,
    Annotation,
    Observation,
    Source,
}

/// Stored comparison is preserved; historical raw re-extraction is separate, read-only material.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotObservation {
    pub record: ObservationAssessment,
    pub reconstructed_definition: Option<Value>,
}
impl SnapshotObservation {
    pub fn definition(&self) -> Option<&Value> {
        self.reconstructed_definition
            .as_ref()
            .or(self.record.incoming_definition.as_ref())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotSource {
    pub event_id: Uuid,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
    pub actor_id: UserId,
    pub producer_id: Uuid,
    pub record_id: Uuid,
    pub ingestion_id: Option<Uuid>,
    pub kind: String,
    pub captured_at: String,
    pub context: Option<CaptureContext>,
    pub raw_hash: Option<String>,
    pub evidence_status: String,
    pub evidence_coverage: Option<Value>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotInputs {
    pub observations: Vec<SnapshotObservation>,
    pub sources: Vec<SnapshotSource>,
    /// Explicit, version-independent diagnostic counts, never a claim that a model has read anything.
    pub gaps: std::collections::BTreeMap<String, u64>,
}
