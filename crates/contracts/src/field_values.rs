//! Typed observations stored once per frozen sample, independent of model prose/input budgets.
use crate::{endpoint::ValueType, knowledge::EvidenceSource};
use nexofolio_common::{EndpointId, ProjectId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldValueArchive {
    pub version: u32,
    pub documents: BTreeMap<String, ValueDocument>,
    /// Endpoint UUID / qualified field key. Each pointer addresses a value in documents.
    pub fields: BTreeMap<String, ObservedField>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueDocument {
    pub source: EvidenceSource,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub observation_key: Option<crate::observation::ObservationKey>,
    pub representation: String,
    pub state: String,
    pub root: Value,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedField {
    pub values: Vec<ObservedValueRef>,
    pub coverage: Vec<ValueCoverage>,
    pub hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValueRef {
    pub id: String,
    pub value_type: ValueType,
    pub representation: String,
    pub occurrences: Vec<ValuePointer>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValuePointer {
    pub document: String,
    pub pointer: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueCoverage {
    pub source: EvidenceSource,
    /// full/partial/truncated/none/unreadable/absent/unavailable; never a guessed value.
    pub state: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldObservationSummary {
    /// Populated by the execution actor after the immutable archive has been saved.
    pub archive_id: Option<uuid::Uuid>,
    pub total: usize,
    pub hash: String,
    pub states: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValuePreview {
    pub id: String,
    pub value_type: ValueType,
    pub representation: String,
    pub preview: String,
    pub preview_truncated: bool,
    pub occurrences: usize,
    pub sources: Vec<ObservedValueSource>,
    pub sources_total: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValuePage {
    pub summary: FieldObservationSummary,
    pub items: Vec<ObservedValuePreview>,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldValueQuery {
    pub project: ProjectId,
    pub archive: uuid::Uuid,
    pub endpoint: EndpointId,
    pub field: String,
    pub offset: usize,
    pub limit: usize,
    #[serde(default)]
    pub environment: Option<nexofolio_common::EnvironmentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedValueSource {
    #[serde(flatten)]
    pub source: EvidenceSource,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub observation_key: Option<crate::observation::ObservationKey>,
}
