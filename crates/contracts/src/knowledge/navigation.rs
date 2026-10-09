//! Explicit, scoped navigation knowledge. A captured correspondence is not a requirement.
use crate::endpoint::FieldLocation;
use nexofolio_common::{EndpointId, EnvironmentId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    #[default]
    Candidate,
    Observed,
    Direct,
    Confirmed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceSource {
    #[serde(default)]
    pub endpoint: Option<EndpointId>,
    pub example: String,
    pub environment: Option<EnvironmentId>,
    pub location: Option<FieldLocation>,
    pub pointer: Option<String>,
    pub page: Option<String>,
    pub sequence: Option<u64>,
    pub platform: Option<String>,
    pub complete: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMeaning {
    pub unit: Option<String>,
    /// A directly captured unit does not confirm model-written details.
    #[serde(default)]
    pub unit_confirmation: Confirmation,
    pub role: Option<String>,
    #[serde(default)]
    pub details: Vec<String>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub confirmation: Confirmation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointContext {
    pub input_hash: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub authentication: Vec<AuthenticationUse>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub sources: Vec<EvidenceSource>,
    pub recipe: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationUse {
    pub carrier: String,
    pub scheme: String,
    /// Opaque reference only, never a reusable captured credential.
    pub credential_ref: String,
    pub source: EvidenceSource,
    pub confirmation: Confirmation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkContext {
    /// Captured source request field equal to the response value: a detail lookup input, not discovery.
    #[serde(default)]
    pub source_input: Option<InputRef>,
    pub from_location: Option<FieldLocation>,
    pub to_location: Option<FieldLocation>,
    pub environment: Option<EnvironmentId>,
    pub scope: Option<String>,
    pub condition: Option<String>,
    #[serde(default)]
    pub confirmation: Confirmation,
    #[serde(default)]
    pub sources: Vec<EvidenceSource>,
    #[serde(default)]
    pub matched_fields: Vec<FieldMapping>,
    pub recipe: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRef {
    pub endpoint: EndpointId,
    pub location: FieldLocation,
    pub path: String,
}

/// Complete relation identity, shared by inference, updates and persistence.
/// Missing legacy locations remain unknown, never guessed from a field name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkIdentity {
    pub from: EndpointId,
    pub from_field: Option<String>,
    pub from_location: Option<FieldLocation>,
    pub to: EndpointId,
    pub to_field: Option<String>,
    pub to_location: Option<FieldLocation>,
    pub relation: super::Relation,
    pub environment: Option<EnvironmentId>,
}

impl LinkIdentity {
    pub fn new(
        from: EndpointId,
        from_field: Option<String>,
        to: EndpointId,
        to_field: Option<String>,
        relation: super::Relation,
        context: &LinkContext,
    ) -> Self {
        Self {
            from,
            from_field,
            to,
            to_field,
            relation,
            from_location: context.from_location,
            to_location: context.to_location,
            environment: context.environment,
        }
    }
}

impl super::Link {
    pub fn identity(&self) -> LinkIdentity {
        LinkIdentity::new(
            self.from,
            self.from_field.clone(),
            self.to,
            self.to_field.clone(),
            self.relation,
            &self.context,
        )
    }
}
