//! Durable structural findings; an observation field is not a field in an adopted revision.
use crate::{EnvironmentId, InterfaceId, ProjectId, RevisionId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
pub const ASSESSMENT_RULE_VERSION: &str = "observed-comparison-2";
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentKind {
    Duplicate,
    Enrichment,
    Difference,
    Insufficient,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuralFinding {
    pub kind: AssessmentKind,
    pub location: String,
    pub path: String,
    pub reason: String,
    pub before: Option<Value>,
    pub observed: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StructuralAssessment {
    pub rule_version: String,
    pub initial: bool,
    pub categories: Vec<AssessmentKind>,
    pub findings: Vec<StructuralFinding>,
}
impl StructuralAssessment {
    pub fn new(initial: bool, findings: Vec<StructuralFinding>) -> Self {
        let mut unique = Vec::with_capacity(findings.len());
        for finding in findings {
            if !unique.contains(&finding) {
                unique.push(finding);
            }
        }
        let findings = unique;
        let mut categories: Vec<_> = findings.iter().map(|f| f.kind).collect();
        categories.sort();
        categories.dedup();
        if categories.is_empty() && !initial {
            categories.push(AssessmentKind::Duplicate);
        }
        Self {
            rule_version: ASSESSMENT_RULE_VERSION.into(),
            initial,
            categories,
            findings,
        }
    }
    pub fn is_duplicate(&self) -> bool {
        !self.initial && self.categories == [AssessmentKind::Duplicate]
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationAssessment {
    pub project_id: ProjectId,
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    /// Source of the incoming structure, not a claim that new fields belong to base_revision_id.
    pub ingestion_id: Uuid,
    pub base_revision_id: Option<RevisionId>,
    pub extractor_version: Option<String>,
    /// None means historical data not assessed by this rule; it never means duplicate.
    pub assessment: Option<StructuralAssessment>,
    pub incoming_definition: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationAssessmentPage {
    pub items: Vec<ObservationAssessment>,
    pub page: u32,
    pub limit: u32,
    pub total: i64,
}

/// Field observed in an input, deliberately without an adopted revision ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationFieldRef {
    pub project_id: ProjectId,
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    pub ingestion_id: Uuid,
    pub location: String,
    pub path: String,
}
impl ObservationAssessment {
    pub fn observed_field(&self, finding: &StructuralFinding) -> ObservationFieldRef {
        ObservationFieldRef {
            project_id: self.project_id,
            interface_id: self.interface_id,
            environment_id: self.environment_id,
            ingestion_id: self.ingestion_id,
            location: finding.location.clone(),
            path: finding.path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_samples_keep_unassessed_distinct_and_never_borrow_a_revision() {
        let mixed: ObservationAssessment = serde_json::from_str(include_str!(
            "../../../contracts/documents/fixtures/assessment-mixed.json"
        ))
        .unwrap();
        assert_eq!(
            mixed.assessment.as_ref().unwrap().categories,
            [
                AssessmentKind::Enrichment,
                AssessmentKind::Difference,
                AssessmentKind::Insufficient
            ]
        );
        let historical: ObservationAssessment = serde_json::from_str(include_str!(
            "../../../contracts/documents/fixtures/assessment-historical.json"
        ))
        .unwrap();
        assert!(historical.assessment.is_none());
        let reference: ObservationFieldRef = serde_json::from_str(include_str!(
            "../../../contracts/documents/fixtures/observed-field-ref.json"
        ))
        .unwrap();
        let mut forbidden = serde_json::to_value(reference).unwrap();
        forbidden["revision_id"] = serde_json::json!(mixed.base_revision_id);
        assert!(serde_json::from_value::<ObservationFieldRef>(forbidden).is_err());
    }
}

/// Evidence may describe an unadopted field. Exactly one basis is populated by the extractor.
/// Adopted references serialize as the existing FieldRef; observation references never claim a revision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceFieldRef {
    pub interface_id: InterfaceId,
    pub environment_id: EnvironmentId,
    pub location: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub revision_id: Option<RevisionId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub observation: Option<ObservationFieldRef>,
}
impl From<crate::FieldRef> for EvidenceFieldRef {
    fn from(f: crate::FieldRef) -> Self {
        Self {
            interface_id: f.interface_id,
            environment_id: f.environment_id,
            location: f.location,
            path: f.path,
            revision_id: Some(f.revision_id),
            observation: None,
        }
    }
}

impl EvidenceFieldRef {
    pub fn at(&self, location: String, path: String) -> Self {
        let mut field = self.clone();
        field.location = location.clone();
        field.path = path.clone();
        if let Some(source) = &mut field.observation {
            source.location = location;
            source.path = path;
        }
        field
    }
    pub fn adopted(&self) -> Option<crate::FieldRef> {
        if self.observation.is_some() {
            return None;
        }
        Some(crate::FieldRef {
            interface_id: self.interface_id,
            environment_id: self.environment_id,
            revision_id: self.revision_id?,
            location: self.location.clone(),
            path: self.path.clone(),
        })
    }
}
impl PartialEq<crate::FieldRef> for EvidenceFieldRef {
    fn eq(&self, other: &crate::FieldRef) -> bool {
        self.adopted().as_ref() == Some(other)
    }
}
