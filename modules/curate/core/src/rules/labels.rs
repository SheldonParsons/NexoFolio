//! Direct column label bindings are scoped to the same captured page and environment.
use nexofolio_contracts::knowledge::{EvidenceKind, FieldEvidence, FieldOption};
use sha2::{Digest, Sha256};

pub(super) fn evaluate(context: &super::RuleContext<'_>) -> Vec<FieldEvidence> {
    if context.own.iter().all(|s| {
        !matches!(
            s.location,
            nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. }
        ) || s.row.is_empty()
    }) {
        return vec![];
    }
    let key_hash = format!("{:x}", Sha256::digest(context.name.as_bytes()));
    let mut out = Vec::new();
    for code in context.index.named_value("code", &key_hash).filter(|s| {
        s.name == "code"
            && s.value == key_hash
            && context.own.iter().any(|own| {
                own.environment == s.environment
                    && own.source.page.is_some()
                    && scoped_record(context, own, s)
            })
    }) {
        let Some(label) = context.index.record(code).find(|s| {
            s.endpoint == code.endpoint
                && s.example == code.example
                && s.row == code.row
                && ["title", "label", "name", "columnName"].contains(&s.name.as_str())
                && s.value
                    .chars()
                    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        }) else {
            continue;
        };
        out.push(FieldEvidence {
            kind: EvidenceKind::DirectSource,
            text: "列配置 code 与字段键 SHA-256 一致；同页面或由至少两个实体标识共同匹配到配置页面，保留该实体范围".into(),
            endpoint: code.endpoint,
            field: label.path.clone(),
            example: Some(label.example.clone()),
            environment: Some(label.environment),
            options: vec![FieldOption {
                value: context.name.into(),
                label: Some(label.value.clone()),
            }],
            source: Some(label.source.clone()),
        });
    }
    // Keep independent pointers for the field and the row that established label scope.
    if !out.is_empty() {
        for own in context.own.iter().take(2) {
            out.push(FieldEvidence {
                kind: EvidenceKind::Observation,
                text: "中文列标签适用范围的目标记录；通过同条实体标识核对，而非全局英文名覆盖"
                    .into(),
                endpoint: own.endpoint,
                field: own.path.clone(),
                example: Some(own.example.clone()),
                environment: Some(own.environment),
                options: vec![],
                source: Some(own.source.clone()),
            });
        }
        let pages: Vec<_> = out
            .iter()
            .filter(|e| e.kind == EvidenceKind::DirectSource)
            .filter_map(|e| e.source.as_ref().and_then(|s| s.page.clone()))
            .collect();
        for own in context.own.iter().take(2) {
            for bridge in context
                .index
                .named_value(&own.name, &own.value)
                .filter(|s| {
                    s.name == own.name
                        && s.value == own.value
                        && s.environment == own.environment
                        && pages.contains(&s.source.page.clone().unwrap_or_default())
                })
                .take(2)
            {
                out.push(FieldEvidence {
                    kind: EvidenceKind::Observation,
                    text: "配置页面中的对应实体字段；用于核对标签适用范围，不声明调用前置".into(),
                    endpoint: bridge.endpoint,
                    field: bridge.path.clone(),
                    example: Some(bridge.example.clone()),
                    environment: Some(bridge.environment),
                    options: vec![],
                    source: Some(bridge.source.clone()),
                });
            }
        }
    }
    out.truncate(12);
    out
}

fn scoped_record(
    context: &super::RuleContext<'_>,
    own: &crate::SampleField,
    column: &crate::SampleField,
) -> bool {
    if own.source.page == column.source.page {
        return true;
    }
    // Bridge a label only through the same entity row on the configured page,
    // with at least two identifying fields. A shared amount/type is not enough.
    let identity: Vec<_> = context
        .index
        .record(own)
        .filter(|s| {
            s.endpoint == own.endpoint
                && s.example == own.example
                && s.row == own.row
                && ["id", "itemCode", "code"].contains(&s.name.as_str())
                && !["0", "1"].contains(&s.value.as_str())
        })
        .collect();
    let mut groups = std::collections::BTreeMap::new();
    for id in identity {
        for bridge in context.index.named_value(&id.name, &id.value).filter(|s| {
            s.environment == column.environment
                && s.source.page == column.source.page
                && same_family(
                    context.source_path(s.endpoint),
                    &context.facts.summary.path_template,
                )
                && s.name == id.name
                && s.value == id.value
                && matches!(
                    s.location,
                    nexofolio_contracts::endpoint::FieldLocation::ResponseBody { .. }
                )
        }) {
            groups
                .entry((
                    bridge.endpoint.to_string(),
                    bridge.example.clone(),
                    bridge.row.clone(),
                ))
                .or_insert_with(std::collections::BTreeSet::new)
                .insert(id.name.clone());
        }
    }
    groups.values().any(|names| names.len() >= 2)
}

fn same_family(left: &str, right: &str) -> bool {
    !left.is_empty()
        && left.split('/').take(3).collect::<Vec<_>>()
            == right.split('/').take(3).collect::<Vec<_>>()
}
