//! Endpoint and field interpretation contracts, separate from catalogue/history operations.
use super::*;

/// The words for one endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointNote {
    pub endpoint: EndpointId,
    /// Human-readable, e.g. 技术咨询分页查询.
    pub name: Option<String>,
    /// One line: what it is for. This is what the catalogue pass reads, so it
    /// must stand alone without the field tree.
    pub purpose: Option<String>,
    #[serde(default)]
    pub context: EndpointContext,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// One field's interpretation. New curation uses [`field_note_key`] for `path`;
/// the original observed path and location remain in `context`. Legacy notes
/// with an unqualified path can still be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldNote {
    pub endpoint: EndpointId,
    pub path: String,
    pub text: String,
    /// Legacy/model preview only. Complete typed values use context.observations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default)]
    pub context: FieldContext,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// Separately sourced knowledge. More evidence kinds can be added without
/// replacing the human-readable summary or losing earlier observations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<Box<FieldCompletion>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<crate::field_values::FieldObservationSummary>,
    /// Observed structure is machine-readable; model prose stays in meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<FieldStructureContext>,
    pub location: Option<crate::endpoint::FieldLocation>,
    pub field_path: Option<String>,
    pub input_hash: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub display_name_confirmation: Confirmation,
    #[serde(default)]
    pub meaning: FieldMeaning,
    #[serde(default)]
    pub evidence: Vec<FieldEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldStructureContext {
    pub types: Vec<crate::endpoint::ValueType>,
    pub parent: Option<String>,
    pub siblings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Inference,
    Observation,
    RelatedInterface,
    DirectSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldEvidence {
    pub kind: EvidenceKind,
    pub text: String,
    pub endpoint: EndpointId,
    pub field: String,
    pub example: Option<String>,
    pub environment: Option<EnvironmentId>,
    #[serde(default)]
    pub options: Vec<FieldOption>,
    #[serde(default)]
    pub source: Option<EvidenceSource>,
}

/// Values offered by a related interface, not an asserted exhaustive enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOption {
    pub value: String,
    pub label: Option<String>,
}

/// Request and response fields with the same path must not share a note.
pub fn field_note_key(field: &crate::endpoint::FieldFacts) -> String {
    use crate::endpoint::FieldLocation;
    let location = match field.location {
        FieldLocation::Path => "path".into(),
        FieldLocation::Query => "query".into(),
        FieldLocation::RequestHeader => "request_header".into(),
        FieldLocation::ResponseHeader { status } => format!("response_header:{status}"),
        FieldLocation::RequestBody => "request_body".into(),
        FieldLocation::ResponseBody { status } => format!("response_body:{status}"),
    };
    format!("{location}:{}", field.path)
}
