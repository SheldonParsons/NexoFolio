use crate::rules::{self, RuleContext};
use crate::{LinkProposal, SampleField, SignalIndex};
use nexofolio_contracts::endpoint::{FieldLocation, ValueType};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary, FieldPath, PathSegment},
    knowledge::{EvidenceKind, FieldEvidence, FieldOption, field_note_key},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInput {
    pub key: String,
    pub path: String,
    pub location: FieldLocation,
    pub types: Vec<ValueType>,
    /// Legacy bounded prose preview, not the complete observed-value catalogue.
    pub values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observations: Option<nexofolio_contracts::field_values::FieldObservationSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_examples: Vec<serde_json::Value>,
    pub evidence: Vec<FieldEvidence>,
    pub parent: String,
    pub siblings: Vec<String>,
}

pub fn field_inputs(
    facts: &EndpointFacts,
    signals: &[SampleField],
    summaries: &[EndpointSummary],
) -> Vec<FieldInput> {
    let mut rule_facts = vec![facts.clone()];
    rule_facts.extend(
        summaries
            .iter()
            .filter(|s| s.id != facts.summary.id)
            .map(|summary| EndpointFacts {
                requests: vec![],
                summary: summary.clone(),
                fields: vec![],
                addresses: vec![],
                aliases: vec![],
                examples: vec![],
            }),
    );
    let index = SignalIndex::new(signals);
    let links = crate::rules::navigation_links_indexed(&rule_facts, &index);
    indexed_inputs(facts, &index, summaries, &links)
}

pub(crate) fn indexed_inputs(
    facts: &EndpointFacts,
    index: &SignalIndex<'_>,
    summaries: &[EndpointSummary],
    links: &[LinkProposal],
) -> Vec<FieldInput> {
    let mut siblings: HashMap<_, Vec<String>> = HashMap::new();
    for field in &facts.fields {
        let path = field.path.to_string();
        siblings
            .entry((field.location, parent_segments(&field.path)))
            .or_default()
            .push(path);
    }
    facts
        .fields
        .iter()
        .map(|field| {
            let path = field.path.to_string();
            let own: Vec<_> = index
                .field(facts.summary.id, field.location, &path)
                .collect();
            let mut input = FieldInput {
                key: field_note_key(field),
                observations: None,
                value_examples: vec![],
                path: path.clone(),
                location: field.location,
                types: field.types.clone(),
                values: own
                    .iter()
                    .map(|s| s.value.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .take(12)
                    .collect(),
                evidence: observations(&own),
                parent: parent_path(&field.path),
                siblings: siblings
                    .get(&(field.location, parent_segments(&field.path)))
                    .into_iter()
                    .flat_map(|s| s.iter())
                    .take(24)
                    .cloned()
                    .collect(),
            };
            input
                .evidence
                .extend(crate::evidence::header_evidence(facts, field));
            let name = field
                .path
                .0
                .iter()
                .rev()
                .find_map(|segment| match segment {
                    PathSegment::Key(name) => Some(name.as_str()),
                    _ => None,
                })
                .unwrap_or("");
            input.evidence.extend(rules::related(&RuleContext {
                facts,
                name,
                own: &own,
                signals: index.all(),
                index,
                summaries,
            }));
            for link in links.iter().filter(|l| {
                l.to == facts.summary.id
                    && l.to_field == input.path
                    && l.context.to_location == Some(input.location)
            }) {
                input.evidence.push(FieldEvidence {
                    kind: EvidenceKind::RelatedInterface,
                    text: link.evidence.clone(),
                    endpoint: link.from,
                    field: link.from_field.clone(),
                    example: link
                        .context
                        .sources
                        .iter()
                        .find(|s| s.endpoint == Some(link.from))
                        .map(|s| s.example.clone()),
                    environment: link.context.environment,
                    options: vec![],
                    source: link
                        .context
                        .sources
                        .iter()
                        .find(|s| s.endpoint == Some(link.from))
                        .cloned(),
                });
            }
            input.evidence.truncate(24);
            input
        })
        .collect()
}

fn observations(own: &[&SampleField]) -> Vec<FieldEvidence> {
    let mut evidence = Vec::new();
    for sample in own {
        if evidence.iter().any(|e: &FieldEvidence| {
            e.example.as_deref() == Some(&sample.example)
                && e.environment == Some(sample.environment)
        }) {
            continue;
        }
        evidence.push(FieldEvidence {
            kind: EvidenceKind::Observation,
            text: "真实调用样本中的观测值；不是完整枚举或业务约束".into(),
            endpoint: sample.endpoint,
            field: sample.path.clone(),
            example: Some(sample.example.clone()),
            environment: Some(sample.environment),
            source: Some(sample.source.clone()),
            options: own
                .iter()
                .filter(|s| s.example == sample.example && s.environment == sample.environment)
                .take(12)
                .map(|s| FieldOption {
                    value: s.value.clone(),
                    label: None,
                })
                .collect(),
        });
    }
    evidence
}

fn parent_segments(path: &FieldPath) -> Vec<PathSegment> {
    path.0[..path.0.len().saturating_sub(1)].to_vec()
}
fn parent_path(path: &FieldPath) -> String {
    FieldPath(parent_segments(path)).to_string()
}
