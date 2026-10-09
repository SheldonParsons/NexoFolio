use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::{field_inputs, standard_field_command};
fn facts(location: FieldLocation, path: Vec<PathSegment>, types: Vec<ValueType>) -> EndpointFacts {
    EndpointFacts {
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: ProjectId::new(),
            method: "GET".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        requests: vec![],
        fields: vec![FieldFacts {
            location,
            path: FieldPath(path),
            types,
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        }],
        aliases: vec![],
        addresses: vec![],
        examples: vec![],
    }
}
#[test]
fn standard_headers_keep_values_scope_and_do_not_assert_business_requirements() {
    let facts = facts(
        FieldLocation::RequestHeader,
        vec![PathSegment::Key("Authorization".into())],
        vec![ValueType::String],
    );
    let mut input = field_inputs(&facts, &[], &[]).remove(0);
    input.values = vec!["fixture-only".into()];
    let command = standard_field_command(&facts, &EndpointKnowledge::default(), &input).unwrap();
    let Command::PutFieldNote {
        context,
        values,
        text,
        ..
    } = command
    else {
        panic!()
    };
    assert_eq!(values, input.values);
    assert_eq!(context.location, Some(FieldLocation::RequestHeader));
    assert_eq!(context.display_name_confirmation, Confirmation::Candidate);
    assert!(text.contains("历史值不代表当前有效"));
    assert!(!context.meaning.unknowns.is_empty());
    assert!(context.evidence.last().unwrap().text.contains("rfc9110"));
}
#[test]
fn custom_fields_and_business_evidence_still_require_interpretation() {
    for (location, path, types) in [
        (
            FieldLocation::ResponseHeader { status: 200 },
            vec![PathSegment::Key("traceid".into())],
            vec![ValueType::String],
        ),
        (
            FieldLocation::ResponseBody { status: 200 },
            vec![PathSegment::Key("date".into())],
            vec![ValueType::String],
        ),
        (
            FieldLocation::ResponseBody { status: 200 },
            vec![],
            vec![ValueType::Boolean],
        ),
    ] {
        let facts = facts(location, path, types);
        let input = field_inputs(&facts, &[], &[]).remove(0);
        assert!(standard_field_command(&facts, &EndpointKnowledge::default(), &input).is_none());
    }
    let facts = facts(
        FieldLocation::ResponseHeader { status: 200 },
        vec![PathSegment::Key("Content-Type".into())],
        vec![ValueType::String],
    );
    let mut input = field_inputs(&facts, &[], &[]).remove(0);
    input.evidence.push(FieldEvidence {
        kind: EvidenceKind::RelatedInterface,
        text: "fixture business constraint".into(),
        endpoint: facts.summary.id,
        field: input.path.clone(),
        example: None,
        environment: None,
        options: vec![],
        source: None,
    });
    assert!(standard_field_command(&facts, &EndpointKnowledge::default(), &input).is_none());
}
#[test]
fn container_root_is_structural_but_existing_partial_knowledge_is_protected() {
    let facts = facts(
        FieldLocation::ResponseBody { status: 200 },
        vec![],
        vec![ValueType::Object],
    );
    let input = field_inputs(&facts, &[], &[]).remove(0);
    let command = standard_field_command(&facts, &EndpointKnowledge::default(), &input).unwrap();
    let Command::PutFieldNote {
        context,
        text,
        values,
        path,
        endpoint,
    } = command
    else {
        panic!()
    };
    assert!(
        context
            .completion
            .as_ref()
            .unwrap()
            .addressed(context.display_name.as_deref(), &text)
    );
    let knowledge = EndpointKnowledge {
        fields: vec![FieldNote {
            endpoint,
            path,
            text,
            values,
            context,
            author: Author::Curate {
                pass: "fields".into(),
                model: Some("fixture".into()),
            },
            at: "2026-10-09T00:00:00Z".parse().unwrap(),
        }],
        ..Default::default()
    };
    assert!(standard_field_command(&facts, &knowledge, &input).is_none());
}

#[test]
fn same_header_coincidence_is_retained_without_becoming_a_business_dependency() {
    let facts = facts(
        FieldLocation::ResponseHeader { status: 200 },
        vec![PathSegment::Key("content-type".into())],
        vec![ValueType::String],
    );
    let mut input = field_inputs(&facts, &[], &[]).remove(0);
    let clue = FieldEvidence {
        kind: EvidenceKind::RelatedInterface,
        text: "与 /fixture 的 content-type 观测到同名同值；候选关联，未确认值来源或调用依赖".into(),
        endpoint: EndpointId::new(),
        field: input.path.clone(),
        example: Some("fixture-sample".into()),
        environment: None,
        options: vec![],
        source: Some(EvidenceSource {
            example: "fixture-sample".into(),
            location: Some(input.location),
            ..Default::default()
        }),
    };
    input.evidence.push(clue.clone());
    let Some(Command::PutFieldNote { context, .. }) =
        standard_field_command(&facts, &EndpointKnowledge::default(), &input)
    else {
        panic!()
    };
    assert!(context.evidence.contains(&clue));
    assert!(
        context
            .meaning
            .unknowns
            .iter()
            .any(|s| s.contains("候选关联"))
    );
    input.evidence[0].source.as_mut().unwrap().location =
        Some(FieldLocation::ResponseBody { status: 200 });
    assert!(standard_field_command(&facts, &EndpointKnowledge::default(), &input).is_none());
}
