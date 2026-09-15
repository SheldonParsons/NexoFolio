//! Additive capture transport. Payload contracts are assembled with the existing HTTP schema.
use crate::{
    Environment, EnvironmentId, EnvironmentRef, InterfaceId, ProjectId, RevisionId, UserId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    HttpExchange,
    PageContext,
    Interaction,
    UiSnapshot,
    ImageReference,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureContext {
    pub browser_instance_id: Uuid,
    pub page_instance_id: Uuid,
    pub frame_instance_id: Uuid,
    pub view_id: Uuid,
    pub event_seq: u64,
    pub page_url: String,
    #[serde(default)]
    pub interaction_id: Option<Uuid>,
    #[serde(default)]
    pub request_started_at_ms: Option<i64>,
    #[serde(default)]
    pub response_completed_at_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureProducer {
    pub r#type: String,
    pub instance_id: Uuid,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureBatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_key: Option<String>,
    pub schema_version: String,
    pub batch_id: Uuid,
    pub project_id: ProjectId,
    pub environment: EnvironmentRef,
    pub source: CaptureProducer,
    pub records: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureRecord {
    pub record_id: Uuid,
    pub kind: CaptureKind,
    pub payload_version: String,
    pub captured_at: String,
    #[serde(default)]
    pub context: Option<CaptureContext>,
    pub payload: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiValueState {
    Present,
    Omitted,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiValue {
    pub state: UiValueState,
    pub value: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiOption {
    pub label: String,
    pub value: UiValue,
    pub selected: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElementBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiElement {
    pub element_id: String,
    pub tag: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    pub value: UiValue,
    pub options: Vec<UiOption>,
    #[serde(default)]
    pub bounds: Option<ElementBounds>,
    pub visible: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageContextPayload {
    pub url: String,
    pub title: String,
    pub breadcrumbs: Vec<String>,
    pub regions: Vec<String>,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionAction {
    Click,
    Change,
    Submit,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InteractionPayload {
    pub action: InteractionAction,
    pub target: UiElement,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiSnapshotPayload {
    pub elements: Vec<UiElement>,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageReferencePayload {
    pub asset_id: Uuid,
    #[serde(default)]
    pub trigger_record_id: Option<Uuid>,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub device_pixel_ratio: f64,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureReceiptStatus {
    Accepted,
    Ignored,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureReceipt {
    pub record_index: usize,
    pub record_id: Option<Uuid>,
    pub status: CaptureReceiptStatus,
    pub reason_code: String,
    pub retryable: bool,
    pub observation_id: Option<Uuid>,
    pub ingestion_id: Option<Uuid>,
    pub structure: String,
    pub replayed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureBatchReceipt {
    pub schema_version: String,
    pub batch_id: Uuid,
    pub environment: Option<Environment>,
    pub results: Vec<CaptureReceipt>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssetReceipt {
    pub asset_id: Uuid,
    pub project_id: ProjectId,
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
    pub replayed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct FieldRef {
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    pub revision_id: RevisionId,
    pub location: String,
    pub path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceFact {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
    pub kind: String,
    pub subject: Value,
    pub data: Value,
    pub observations: i64,
    pub first_seen: String,
    pub last_seen: String,
    pub samples: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidencePage {
    pub items: Vec<EvidenceFact>,
    pub page: u32,
    pub limit: u32,
    pub total: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureObservation {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
    pub kind: CaptureKind,
    pub record_id: Uuid,
    pub actor_id: UserId,
    pub captured_at: String,
    pub context: Option<CaptureContext>,
    pub structure: String,
    pub ingestion_id: Option<Uuid>,
    pub evidence_status: String,
    pub error_code: Option<String>,
    pub payload: Option<Value>,
    pub payload_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureLimits {
    pub max_records: u32,
    pub max_batch_bytes: u64,
    pub max_record_bytes: u64,
    pub max_concurrent_batches: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureFlush {
    pub max_records: u32,
    pub max_batch_bytes: u64,
    pub max_wait_ms: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureAssetsCapability {
    pub max_bytes: u64,
    pub media_types: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureCapabilities {
    pub schema_versions: Vec<String>,
    pub payload_versions: std::collections::BTreeMap<String, Vec<String>>,
    pub limits: CaptureLimits,
    pub flush_defaults: CaptureFlush,
    pub content_encodings: Vec<String>,
    pub assets: CaptureAssetsCapability,
}
