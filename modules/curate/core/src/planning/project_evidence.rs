//! One project graph and evidence index, shared by every endpoint projection.
use crate::{FieldInput, LinkProposal, SampleField, SignalIndex};
use nexofolio_contracts::endpoint::{EndpointFacts, EndpointSummary};
pub struct ProjectEvidence<'a> {
    index: SignalIndex<'a>,
    summaries: Vec<EndpointSummary>,
    links: Vec<LinkProposal>,
}
impl<'a> ProjectEvidence<'a> {
    pub fn new(facts: &[EndpointFacts], signals: &'a [SampleField]) -> Self {
        Self::from_summaries(facts.iter().map(|f| f.summary.clone()).collect(), signals)
    }
    /// Global rules consume complete signals and identities; endpoint bodies are projected on demand.
    pub fn from_summaries(summaries: Vec<EndpointSummary>, signals: &'a [SampleField]) -> Self {
        let index = SignalIndex::new(signals);
        let links = crate::rules::navigation_links_for_summaries(&summaries, &index);
        Self {
            index,
            links,
            summaries,
        }
    }
    pub fn fields(&self, facts: &EndpointFacts) -> Vec<FieldInput> {
        super::inputs::indexed_inputs(facts, &self.index, &self.summaries, &self.links)
    }
    pub fn links(&self) -> &[LinkProposal] {
        &self.links
    }
}
