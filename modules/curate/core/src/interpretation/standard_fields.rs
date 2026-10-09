//! Generate bounded generic notes without a model; source facts remain attached individually.
use crate::{FieldInput, field_input_hash};
use nexofolio_contracts::{endpoint::EndpointFacts, knowledge::*};
pub fn standard_field_command(
    facts: &EndpointFacts,
    knowledge: &EndpointKnowledge,
    input: &FieldInput,
) -> Option<Command> {
    // Existing accepted or partial text is never silently replaced by a generic rule.
    if knowledge
        .fields
        .iter()
        .any(|n| n.path == input.key || (n.context.location.is_none() && n.path == input.path))
    {
        return None;
    }
    let rule = crate::rules::standard_fields::standard_meaning(input)?;
    let mut evidence = input.evidence.clone();
    evidence.push(FieldEvidence {
        kind: EvidenceKind::Inference,
        text: format!(
            "依据通用规范或结构规则补齐基础含义：{}；不构成该接口业务约束的确认。",
            rule.reference
        ),
        endpoint: facts.summary.id,
        field: input.path.clone(),
        example: None,
        environment: None,
        options: vec![],
        source: None,
    });
    let mut unknowns = vec![];
    if input
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::RelatedInterface)
    {
        unknowns.push("同名同值仅为候选关联，未确认值来源或调用依赖".into());
    }
    if matches!(
        input.location,
        nexofolio_contracts::endpoint::FieldLocation::RequestHeader
    ) {
        unknowns.push("是否必需及接口特有约束须由业务资料确认".into());
    }
    if input.observations.as_ref().is_some_and(|o| {
        o.states.keys().any(|k| {
            matches!(
                k.as_str(),
                "partial" | "truncated" | "unreadable" | "unavailable" | "unreadable_structure"
            )
        })
    }) {
        unknowns.push("存在未完整采集的资料，完整性以真实样本状态为准".into());
    }
    Some(Command::PutFieldNote {
        endpoint: facts.summary.id,
        path: input.key.clone(),
        text: rule.text.into(),
        values: input.values.clone(),
        context: FieldContext {
            location: Some(input.location),
            field_path: Some(input.path.clone()),
            input_hash: Some(field_input_hash(facts, knowledge.note.as_ref(), input)),
            display_name: Some(rule.name.into()),
            display_name_confirmation: Confirmation::Candidate,
            completion: Some(Box::new(FieldCompletion {
                name: AspectCompletion::filled(),
                description: AspectCompletion::filled(),
            })),
            observations: input.observations.clone(),
            evidence,
            structure: Some(FieldStructureContext {
                types: input.types.clone(),
                parent: (!input.parent.is_empty()).then(|| input.parent.clone()),
                siblings: input
                    .siblings
                    .iter()
                    .filter(|p| *p != &input.path)
                    .cloned()
                    .collect(),
            }),
            meaning: FieldMeaning {
                unknowns,
                ..Default::default()
            },
        },
    })
}
