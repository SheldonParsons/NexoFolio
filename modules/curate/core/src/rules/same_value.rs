//! Same-name/value overlaps are candidate associations, never call dependencies.
use super::RuleContext;
use nexofolio_contracts::knowledge::{EvidenceKind, FieldEvidence};

pub(super) fn evaluate(context: &RuleContext<'_>) -> Vec<FieldEvidence> {
    let mut evidence = Vec::new();
    if ![
        "", "id", "code", "status", "message", "msg", "size", "sort", "page", "total", "success",
        "name", "type",
    ]
    .contains(&context.name.to_ascii_lowercase().as_str())
    {
        let mut sources = std::collections::HashSet::new();
        for other in context.index.name(context.name).filter(|s| {
            s.endpoint != context.facts.summary.id
                && s.name == context.name
                && context.own.iter().any(|own| {
                    own.environment == s.environment
                        && own.value == s.value
                        && !["0", "1", "true", "false"].contains(&s.value.as_str())
                })
        }) {
            if sources.len() >= 3 {
                break;
            }
            if !sources.insert(other.endpoint) {
                continue;
            }
            let source = context.source_path(other.endpoint);
            evidence.push(FieldEvidence {
                kind: EvidenceKind::RelatedInterface,
                text: format!(
                    "与 {source} 的 {} 观测到同名同值；候选关联，未确认值来源或调用依赖",
                    other.path
                ),
                endpoint: other.endpoint,
                field: other.path.clone(),
                example: Some(other.example.clone()),
                environment: Some(other.environment),
                options: vec![],
                source: Some(other.source.clone()),
            });
        }
    }
    evidence
}
