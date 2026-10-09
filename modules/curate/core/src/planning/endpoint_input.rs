//! Stable, evidence-dependent description inputs; old model text is not evidence.
use crate::{SampleField, field_inputs};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary},
    knowledge::{AuthenticationUse, EndpointContext, EndpointNote},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const DESCRIPTION_RECIPE: &str = "endpoint-knowledge-v2";
#[derive(Debug, Clone, Serialize)]
pub struct EndpointInput {
    pub method: String,
    pub path: String,
    pub fields: Vec<crate::FieldInput>,
    pub authentication: Vec<AuthenticationUse>,
    pub request_material: Vec<serde_json::Value>,
}
pub fn endpoint_input(
    facts: &EndpointFacts,
    signals: &[SampleField],
    summaries: &[EndpointSummary],
    authentication: Vec<AuthenticationUse>,
) -> EndpointInput {
    endpoint_input_from_fields(
        facts,
        field_inputs(facts, signals, summaries),
        authentication,
    )
}
pub fn endpoint_input_from_fields(
    facts: &EndpointFacts,
    all: Vec<crate::FieldInput>,
    authentication: Vec<AuthenticationUse>,
) -> EndpointInput {
    // Select each location independently so request-heavy APIs still expose output.
    let mut fields = Vec::new();
    for group in 0..6 {
        fields.extend(
            all.iter()
                .filter(|f| match f.location {
                    nexofolio_contracts::endpoint::FieldLocation::RequestHeader => group == 4,
                    nexofolio_contracts::endpoint::FieldLocation::ResponseHeader { .. } => {
                        group == 5
                    }
                    nexofolio_contracts::endpoint::FieldLocation::Path => group == 0,
                    nexofolio_contracts::endpoint::FieldLocation::Query => group == 1,
                    nexofolio_contracts::endpoint::FieldLocation::RequestBody => group == 2,
                    nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. } => group == 3,
                })
                .take(16)
                .cloned(),
        );
    }
    EndpointInput {
        method: facts.summary.method.clone(),
        path: facts.summary.path_template.clone(),
        fields,
        authentication,
        request_material: facts.requests.iter().take(12).map(|r| serde_json::json!({
            "example": r.example_id, "environment": r.environment_id,
            "body_state": serde_json::to_value(&r.request.body).ok().and_then(|b| b.get("state").cloned()),
            "body_root_type": r.body_root_kind,
            "headers_complete": r.request.headers.as_ref().map(|h| h.complete), "gaps": r.gaps
        })).collect(),
    }
}
pub fn endpoint_input_hash(input: &EndpointInput) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(DESCRIPTION_RECIPE, input)).unwrap())
    )
}
pub fn description_pending(input: &EndpointInput, note: Option<&EndpointNote>) -> bool {
    note.is_none_or(|n| {
        !n.author.is_person()
            && (n.name.as_ref().is_none_or(|v| v.trim().is_empty())
                || n.purpose.as_ref().is_none_or(|v| v.trim().is_empty())
                || n.context.input_hash.as_deref() != Some(&endpoint_input_hash(input)))
    })
}
pub fn description_context(input: &EndpointInput) -> EndpointContext {
    let sources = input
        .fields
        .iter()
        .flat_map(|f| &f.evidence)
        .filter_map(|e| e.source.clone())
        .take(12)
        .collect();
    EndpointContext {
        input_hash: Some(endpoint_input_hash(input)),
        recipe: Some(DESCRIPTION_RECIPE.into()),
        authentication: input.authentication.clone(),
        sources,
        unknowns: if input.authentication.is_empty() {
            vec![]
        } else {
            vec!["认证方式为样本使用观测；凭证取得来源及强制要求未确认，不能拼接历史登录链".into()]
        },
        ..Default::default()
    }
}
