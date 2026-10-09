//! Named dictionary selectors and value/label pairs from the same array row.
use super::RuleContext;
use nexofolio_contracts::{
    endpoint::FieldLocation,
    knowledge::{EvidenceKind, FieldEvidence, FieldOption},
};

fn dictionary_name_matches(selector: &str, field: &str) -> bool {
    let normalize = |text: &str| {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let selector = normalize(selector);
    let field = normalize(field);
    !field.is_empty() && (selector == field || (field.len() > 6 && selector.ends_with(&field)))
}

pub(super) fn evaluate(context: &RuleContext<'_>) -> Vec<FieldEvidence> {
    let mut evidence = Vec::new();
    let mut dictionaries = std::collections::HashSet::new();
    for selector in context.signals.iter().filter(|s| {
        s.endpoint != context.facts.summary.id
            && !matches!(s.location, FieldLocation::ResponseBody { .. })
            && ["dictcode", "dicttype", "dictionarycode"]
                .contains(&s.name.to_ascii_lowercase().as_str())
            && dictionary_name_matches(&s.value, context.name)
    }) {
        if !dictionaries.insert((selector.endpoint, selector.environment)) {
            continue;
        }
        let group = context
            .index
            .example(selector)
            .find(|s| {
                s.endpoint == selector.endpoint
                    && s.example == selector.example
                    && matches!(s.location, FieldLocation::ResponseBody { .. })
                    && s.name.eq_ignore_ascii_case("dictCode")
                    && s.value == selector.value
            })
            .map(|s| s.row.as_str());
        let options: Vec<_> = context
            .index
            .example(selector)
            .filter(|s| {
                s.endpoint == selector.endpoint
                    && s.example == selector.example
                    && matches!(s.location, FieldLocation::ResponseBody { .. })
                    && ["value", "code", "dictvalue"]
                        .contains(&s.name.to_ascii_lowercase().as_str())
                    && group.is_none_or(|group| s.row.starts_with(&format!("{group}/")))
            })
            .map(|s| {
                let label = context
                    .index
                    .record(s)
                    .find(|label| {
                        label.endpoint == s.endpoint
                            && label.example == s.example
                            && label.row == s.row
                            && ["label", "name", "dictlabel", "dictname", "text"]
                                .contains(&label.name.to_ascii_lowercase().as_str())
                    })
                    .map(|s| s.value.clone());
                FieldOption {
                    value: s.value.clone(),
                    label,
                }
            })
            .take(20)
            .collect();
        if options.is_empty() {
            continue;
        }
        let source = context.source_path(selector.endpoint);
        evidence.push(FieldEvidence {
            kind: EvidenceKind::RelatedInterface,
            text: format!(
                "字典接口 {source} 的 {} 参数与字段名匹配；提供候选值，适用关系仍待业务确认",
                selector.name
            ),
            endpoint: selector.endpoint,
            field: selector.path.clone(),
            example: Some(selector.example.clone()),
            environment: Some(selector.environment),
            options,
            source: Some(selector.source.clone()),
        });
    }
    evidence
}
