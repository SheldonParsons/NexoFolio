//! Pure relationship judgments. Storage supplies bounded candidate observations.
use crate::{FactDraft, relation_field};
use nexofolio_contracts::EvidenceFieldRef as FieldRef;
use serde_json::json;
pub struct RelationSupport {
    pub alternative_sources: i64,
    pub search_complete: bool,
    pub cross_view_bridge: bool,
    pub interaction_observed: bool,
}
impl RelationSupport {
    pub fn ambiguous(&self) -> bool {
        self.alternative_sources > 1 || !self.search_complete
    }
}
pub fn eligible_relation(source: &FieldRef, target: &FieldRef) -> bool {
    source.interface_id != target.interface_id
        && source.environment_id == target.environment_id
        && relation_field(source)
        && relation_field(target)
}
pub fn parameter_link_fact(
    source: &FieldRef,
    target: &FieldRef,
    transform: &str,
    support: &RelationSupport,
) -> FactDraft {
    FactDraft {
        kind: "parameter_link_candidate".into(),
        subject: json!({"source":source,"target":target}),
        data: json!({"transform":transform,"verification":"inferred","ambiguous":support.ambiguous(),
            "search_complete":support.search_complete,"cross_view_bridge":support.cross_view_bridge,
            "interaction_observed":support.interaction_observed,"complete":false,"support_kind":"value_equality_candidate"}),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_search_or_multiple_sources_never_becomes_unambiguous() {
        let mut support = RelationSupport {
            alternative_sources: 1,
            search_complete: true,
            cross_view_bridge: false,
            interaction_observed: true,
        };
        assert!(!support.ambiguous());
        support.search_complete = false;
        assert!(support.ambiguous());
        support.search_complete = true;
        support.alternative_sources = 2;
        assert!(support.ambiguous());
    }
}
