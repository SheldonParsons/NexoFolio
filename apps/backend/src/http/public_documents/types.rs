//! Allowlisted payloads; full facts, note contexts and samples are never serialized.
use nexofolio_access_contracts::DocumentProject;
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::Confirmation};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct DocumentIndex {
    pub project: DocumentProject,
    #[serde(flatten)]
    pub index: crate::http::document_index::DocumentIndex,
}
#[derive(Serialize)]
pub(super) struct DocumentEndpoint {
    pub retired_source_count: usize,
    pub project_id: ProjectId,
    pub endpoint_id: EndpointId,
    pub environment_id: Option<EnvironmentId>,
    pub method: String,
    pub path: String,
    pub name: Option<String>,
    pub purpose: Option<String>,
    pub description: Option<String>,
    pub unknowns: Vec<String>,
    pub fields: Vec<Field>,
}
#[derive(Serialize)]
pub(super) struct Field {
    pub retired_sources: bool,
    pub key: String,
    pub structure: Structure,
    pub name: Option<String>,
    pub description: Option<String>,
    pub details: Vec<String>,
    pub unknowns: Vec<String>,
    pub confirmation: Option<Confirmation>,
}
#[derive(Serialize)]
pub(super) struct Structure {
    pub location: FieldLocation,
    pub path: FieldPath,
    pub types: Vec<ValueType>,
    pub labels: Vec<EnvironmentLabel>,
    pub differs_between_environments: bool,
    pub declared: Option<DeclaredField>,
    pub conflict: Option<Conflict>,
}
