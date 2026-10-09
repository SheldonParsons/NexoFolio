//! Deterministic evidence rules. Adding a rule does not change model or IO code.
//! Rules emit candidates; they cannot turn observations into confirmed constraints.
mod dictionary;
mod entity_scope;
mod labels;
mod navigation;
mod same_value;
mod template;
pub(crate) use navigation::navigation_links_indexed;
pub use navigation::{LinkProposal, NAVIGATION_RECIPE, navigation_links};

use crate::{SampleField, SignalIndex};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary},
    knowledge::FieldEvidence,
};

pub(super) struct RuleContext<'a> {
    pub facts: &'a EndpointFacts,
    pub name: &'a str,
    pub own: &'a [&'a SampleField],
    pub signals: &'a [SampleField],
    pub index: &'a SignalIndex<'a>,
    pub summaries: &'a [EndpointSummary],
}

impl RuleContext<'_> {
    fn source_path(&self, endpoint: nexofolio_common::EndpointId) -> &str {
        self.summaries
            .iter()
            .find(|s| s.id == endpoint)
            .map(|s| s.path_template.as_str())
            .unwrap_or("")
    }
}

pub(super) fn related(context: &RuleContext<'_>) -> Vec<FieldEvidence> {
    // Ordering is part of the v1 input fingerprint; preserve it on refactors.
    let mut evidence = dictionary::evaluate(context);
    evidence.extend(same_value::evaluate(context));
    evidence.extend(labels::evaluate(context));
    evidence
}

pub(crate) mod standard_fields;
