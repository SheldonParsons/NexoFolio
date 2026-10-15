//! Scoped row correspondences, never automatic hard prerequisites.
use crate::{SampleField, SignalIndex};
use nexofolio_common::EndpointId;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, EndpointSummary, FieldLocation},
    knowledge::{Confirmation, FieldMapping, LinkContext, LinkIdentity, Relation},
};
use std::collections::BTreeMap;

type MatchedRow<'a> = Vec<(&'a SampleField, &'a SampleField)>;

pub const NAVIGATION_RECIPE: &str = "navigation-v3";
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LinkProposal {
    pub from: EndpointId,
    pub from_field: String,
    pub to: EndpointId,
    pub to_field: String,
    pub relation: Relation,
    pub evidence: String,
    pub context: LinkContext,
}

impl LinkProposal {
    pub fn identity(&self) -> LinkIdentity {
        LinkIdentity::new(
            self.from,
            Some(self.from_field.clone()),
            self.to,
            Some(self.to_field.clone()),
            self.relation,
            &self.context,
        )
    }
}

fn request(location: FieldLocation) -> bool {
    !matches!(
        location,
        FieldLocation::ResponseBody { .. } | FieldLocation::ResponseHeader { .. }
    )
}
pub(crate) fn scoped_before(source: &SampleField, target: &SampleField) -> bool {
    source.environment == target.environment
        && source.source.page.is_some()
        && source.source.page == target.source.page
        && source
            .source
            .sequence
            .zip(target.source.sequence)
            .is_some_and(|(a, b)| a < b)
}

fn names_match(source: &str, target: &str) -> bool {
    source == target
        || matches!(
            (source, target),
            ("uniqueId", "id")
                | ("uniqueId", "sliderCaptchaId")
                | ("id", "sliderCaptchaId")
                | ("id", "configId")
                | ("name", "configName")
                | ("id", "opptyProjectId")
                | ("commodityCode", "code")
                | ("commodityModel", "modelOrName")
                | ("model", "modelOrName")
        )
}

fn useful(sample: &SampleField) -> bool {
    ![
        "msg", "message", "size", "sort", "page", "total", "success", "status", "type",
    ]
    .contains(&sample.name.as_str())
        && !["0", "1", "true", "false", "成功", "操作成功"].contains(&sample.value.as_str())
}

pub fn navigation_links(facts: &[EndpointFacts], signals: &[SampleField]) -> Vec<LinkProposal> {
    navigation_links_indexed(facts, &SignalIndex::new(signals))
}
pub(crate) fn navigation_links_indexed(
    facts: &[EndpointFacts],
    index: &SignalIndex<'_>,
) -> Vec<LinkProposal> {
    navigation_links_for_summaries(
        &facts.iter().map(|f| f.summary.clone()).collect::<Vec<_>>(),
        index,
    )
}
pub(crate) fn navigation_links_for_summaries(
    facts: &[EndpointSummary],
    index: &SignalIndex<'_>,
) -> Vec<LinkProposal> {
    let mut out = Vec::new();
    for target in facts {
        let own: Vec<_> = index
            .endpoint(target.id)
            .filter(|s| request(s.location))
            .collect();
        let mut rows: BTreeMap<(String, String, String), MatchedRow<'_>> = BTreeMap::new();
        for input in &own {
            for source in index.value(&input.value).filter(|s| {
                s.endpoint != input.endpoint
                    && !request(s.location)
                    && s.value == input.value
                    && names_match(&s.name, &input.name)
                    && scoped_before(s, input)
            }) {
                rows.entry((
                    source.endpoint.to_string(),
                    source.example.clone(),
                    source.row.clone(),
                ))
                .or_default()
                .push((source, input));
            }
        }
        for pairs in rows.values() {
            let source = pairs[0].0;
            let target_row = &pairs[0].1.row;
            let pairs: Vec<_> = pairs.iter().filter(|(_, b)| &b.row == target_row).collect();
            let distinct: std::collections::BTreeSet<_> =
                pairs.iter().map(|(a, b)| (&a.name, &b.name)).collect();
            let special = pairs.iter().any(|(a, b)| {
                matches!(
                    (a.name.as_str(), b.name.as_str()),
                    ("uniqueId", "id")
                        | ("uniqueId", "sliderCaptchaId")
                        | ("id", "sliderCaptchaId")
                )
            });
            let path_id = pairs
                .iter()
                .any(|(a, b)| a.name == "id" && b.location == FieldLocation::Path)
                && facts
                    .iter()
                    .find(|f| f.id == source.endpoint)
                    .is_some_and(|f| {
                        super::entity_scope::same_resource(&f.path_template, &target.path_template)
                    });
            if !special && !path_id && (distinct.len() < 2 || !pairs.iter().any(|(a, _)| useful(a)))
            {
                // Single same-name/value overlaps remain weak field clues.
                // An entity id repeated in a detail response is a reference, not discovery.
                if !pairs
                    .iter()
                    .any(|(a, b)| a.name == "id" && b.name == "opptyProjectId")
                {
                    continue;
                }
            }
            let mappings: Vec<_> = pairs
                .iter()
                .map(|(a, b)| FieldMapping {
                    from: a.path.clone(),
                    to: b.path.clone(),
                })
                .collect();
            for (a, b) in pairs {
                let source_input = index.example(a).find(|s| {
                    s.endpoint == a.endpoint
                        && s.example == a.example
                        && s.environment == a.environment
                        && request(s.location)
                        && s.value == a.value
                        && names_match(&a.name, &s.name)
                        && useful(s)
                });
                let requires_same_id = source_input.is_some();
                let relation = if requires_same_id {
                    Relation::EntityReference
                } else {
                    Relation::ParameterSource
                };
                let evidence = if requires_same_id {
                    "当前样本实体对应；来源查询自己需要同一值，不能发现未知 ID"
                } else {
                    "同环境同页面较早响应与目标请求的同条记录对应；仅证明当前样本传递，不证明必调"
                };
                let proposal = LinkProposal {
                    from: a.endpoint,
                    from_field: a.path.clone(),
                    to: b.endpoint,
                    to_field: b.path.clone(),
                    relation,
                    evidence: evidence.into(),
                    context: LinkContext {
                        source_input: source_input.map(|s| {
                            nexofolio_contracts::knowledge::InputRef {
                                endpoint: s.endpoint,
                                location: s.location,
                                path: s.path.clone(),
                            }
                        }),
                        from_location: Some(a.location),
                        to_location: Some(b.location),
                        environment: Some(a.environment),
                        scope: a.source.page.clone(),
                        condition: Some(
                            "目标缺少此输入，且来源接口自身输入可取得；适用当前采样页面/环境"
                                .into(),
                        ),
                        confirmation: Confirmation::Observed,
                        sources: [
                            Some(a.source.clone()),
                            Some(b.source.clone()),
                            source_input.map(|s| s.source.clone()),
                        ]
                        .into_iter()
                        .flatten()
                        .collect(),
                        matched_fields: mappings.clone(),
                        recipe: Some(NAVIGATION_RECIPE.into()),
                    },
                };
                out.push(proposal);
            }
        }
    }
    out.extend(super::template::proposals(facts, index));
    merge(out)
}

fn merge(proposals: Vec<LinkProposal>) -> Vec<LinkProposal> {
    let mut merged: Vec<LinkProposal> = Vec::new();
    for proposal in proposals {
        if let Some(old) = merged
            .iter_mut()
            .find(|p| p.identity() == proposal.identity())
        {
            for source in proposal.context.sources {
                if !old.context.sources.contains(&source) {
                    old.context.sources.push(source);
                }
            }
            for mapping in proposal.context.matched_fields {
                if !old.context.matched_fields.contains(&mapping) {
                    old.context.matched_fields.push(mapping);
                }
            }
        } else {
            merged.push(proposal);
        }
    }
    for proposal in &mut merged {
        proposal
            .context
            .sources
            .sort_by_key(|s| serde_json::to_string(s).unwrap());
        proposal
            .context
            .matched_fields
            .sort_by_key(|m| (m.from.clone(), m.to.clone()));
    }
    merged
}
