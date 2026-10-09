use async_trait::async_trait;
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::{
    CompletionError, Completions, Describe, SampleField, field_input_hash, field_inputs,
    pending_fields, sample_fields,
};
use serde_json::json;

fn facts() -> EndpointFacts {
    EndpointFacts {
        requests: vec![],
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: ProjectId::new(),
            method: "GET".into(),
            path_template: "/orders/detail".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        examples: vec![],
        fields: vec![FieldFacts {
            location: FieldLocation::ResponseBody { status: 200 },
            path: FieldPath(vec![PathSegment::Key("status".into())]),
            types: vec![ValueType::String],
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        }],
    }
}
fn signal(
    endpoint: EndpointId,
    environment: EnvironmentId,
    location: FieldLocation,
    name: &str,
    value: &str,
    row: &str,
) -> SampleField {
    SampleField {
        endpoint,
        environment,
        location,
        path: name.into(),
        name: name.into(),
        value: value.into(),
        example: "sample".into(),
        row: row.into(),

        source: Default::default(),
    }
}
struct Model;
#[async_trait]
impl Completions for Model {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        assert!(prompt.contains("基础含义始终属于推断"));
        Ok(r#"{"fields":[{"key":"0","display_name":"合成字段","description":"订单状态，具体业务含义待确认"}]}"#.into())
    }
}

#[tokio::test]
async fn a_new_dictionary_updates_an_old_field_without_a_new_own_structure() {
    let facts = facts();
    let env = EnvironmentId::new();
    let dictionary = EndpointId::new();
    let mut signals = vec![signal(
        facts.summary.id,
        env,
        FieldLocation::ResponseBody { status: 200 },
        "status",
        "2",
        "",
    )];
    let before = field_inputs(&facts, &signals, &[]);
    let pass = Describe::new(Model, "test".into());
    let commands = pass
        .fields(&facts, &EndpointKnowledge::default(), &before)
        .await
        .unwrap();
    let Command::PutFieldNote {
        path,
        text,
        values,
        context,
        ..
    } = commands[0].clone()
    else {
        panic!();
    };
    let mut knowledge = EndpointKnowledge {
        fields: vec![FieldNote {
            endpoint: facts.summary.id,
            path,
            text,
            values,
            context,
            author: pass.author(),
            at: "2026-09-30T00:00:00Z".parse().unwrap(),
        }],
        ..Default::default()
    };
    assert!(pending_fields(&facts, &knowledge, before.clone()).is_empty());
    signals.extend([
        signal(
            dictionary,
            env,
            FieldLocation::Query,
            "dictCode",
            "status",
            "",
        ),
        signal(
            dictionary,
            env,
            FieldLocation::ResponseBody { status: 200 },
            "value",
            "2",
            "/data/0",
        ),
        signal(
            dictionary,
            env,
            FieldLocation::ResponseBody { status: 200 },
            "label",
            "已完成",
            "/data/0",
        ),
    ]);
    let after = field_inputs(&facts, &signals, &[]);
    assert_ne!(
        field_input_hash(&facts, None, &before[0]),
        field_input_hash(&facts, None, &after[0])
    );
    let pending = pending_fields(&facts, &knowledge, after);
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].evidence.last().unwrap().options[0]
            .label
            .as_deref(),
        Some("已完成")
    );
    let updated = pass.fields(&facts, &knowledge, &pending).await.unwrap();
    assert!(
        matches!(&updated[0], Command::PutFieldNote { context, .. } if context.evidence.iter().any(|e| e.endpoint == dictionary && e.kind == EvidenceKind::RelatedInterface))
    );
    knowledge.fields[0].author = Author::Person {
        user: nexofolio_common::UserId::new(),
    };
    assert!(pending_fields(&facts, &knowledge, pending).is_empty());
}

#[test]
fn array_elements_use_the_array_as_parent_not_the_response_envelope() {
    let mut facts = facts();
    facts.fields = vec![
        FieldFacts {
            path: FieldPath(vec![PathSegment::Key("code".into())]),
            ..facts.fields[0].clone()
        },
        FieldFacts {
            path: FieldPath(vec![PathSegment::Key("data".into())]),
            types: vec![ValueType::Array],
            ..facts.fields[0].clone()
        },
        FieldFacts {
            path: FieldPath(vec![PathSegment::Key("data".into()), PathSegment::Items]),
            types: vec![ValueType::Object],
            ..facts.fields[0].clone()
        },
        FieldFacts {
            path: FieldPath(vec![
                PathSegment::Key("data".into()),
                PathSegment::Items,
                PathSegment::Key("id".into()),
            ]),
            ..facts.fields[0].clone()
        },
    ];
    let inputs = field_inputs(&facts, &[], &[]);
    let item = inputs.iter().find(|f| f.path == "data[]").unwrap();
    assert_eq!(item.parent, "data");
    assert_eq!(item.siblings, vec!["data[]"]);
    assert!(
        !inputs
            .iter()
            .find(|f| f.path == "code")
            .unwrap()
            .siblings
            .contains(&"data[]".into())
    );
    assert_eq!(
        inputs
            .iter()
            .find(|f| f.path == "data[].id")
            .unwrap()
            .parent,
        "data[]"
    );
}

#[test]
fn recipe_upgrade_targets_structural_noise_and_changed_evidence_but_reuses_clean_legacy_meaning() {
    let facts = facts();
    let inputs = field_inputs(&facts, &[], &[]);
    let input = &inputs[0];
    let mut knowledge = EndpointKnowledge {
        fields: vec![FieldNote {
            endpoint: facts.summary.id,
            path: input.key.clone(),
            text: "订单状态，具体含义待确认".into(),
            values: vec![],
            author: Author::Curate {
                pass: "describe".into(),
                model: Some("fixture".into()),
            },
            at: "2026-10-07T00:00:00Z".parse().unwrap(),
            context: FieldContext {
                display_name: Some("订单状态".into()),
                input_hash: Some(nexofolio_curate::legacy_field_input_hash(
                    &facts, None, input,
                )),
                meaning: FieldMeaning {
                    details: vec!["当前观测到状态用于筛选".into()],
                    ..Default::default()
                },
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    assert!(pending_fields(&facts, &knowledge, inputs.clone()).is_empty());
    knowledge.fields[0]
        .context
        .meaning
        .details
        .push("位置：response_body".into());
    assert_eq!(pending_fields(&facts, &knowledge, inputs.clone()).len(), 1);
    knowledge.fields[0].context.meaning.details.clear();
    let mut changed = inputs;
    changed[0].values.push("新的观测值".into());
    assert_eq!(pending_fields(&facts, &knowledge, changed).len(), 1);
}

#[test]
fn same_named_request_and_response_fields_have_separate_identities() {
    let facts = facts();
    let mut request = facts.fields[0].clone();
    request.location = FieldLocation::RequestBody;
    assert_ne!(field_note_key(&request), field_note_key(&facts.fields[0]));
}

#[test]
fn sample_extraction_keeps_array_rows_and_bounded_original_values() {
    let mut facts = facts();
    facts.fields = ["code", "label", "accessToken"]
        .map(|name| FieldFacts {
            path: FieldPath(vec![
                PathSegment::Key("data".into()),
                PathSegment::Items,
                PathSegment::Key(name.into()),
            ]),
            ..facts.fields[0].clone()
        })
        .to_vec();
    let raw = json!({"key":{"batch_id":"00000000-0000-0000-0000-000000000001","record_id":"00000000-0000-0000-0000-000000000002"},"project_id":facts.summary.project_id,"environment_id":EnvironmentId::new(),"site":null,"source":{"platform":"fixture","platform_version":null,"source_url":null},"observed_at":"2026-09-30T00:00:00Z","context":null,"fact":{"kind":"exchange","request":{"method":"GET","url":"https://example.test/dict","url_truncated":false,"headers":null,"body":{"state":"none"}},"response":{"status":200,"headers":null,"body":{"state":"full","media_type":"application/json","content":{"encoding":"text","data":"{\"data\":[{\"code\":\"1\",\"label\":\"待处理\",\"accessToken\":\"private\"},{\"code\":\"2\",\"label\":\"完成\"}]}"}}}}});
    let signals = sample_fields(&facts, "example", &raw);
    assert_eq!(signals.len(), 5);
    assert!(
        signals
            .iter()
            .any(|s| s.name == "accessToken" && s.value == "private")
    );
    assert!(signals.iter().any(|s| s.value == "2" && s.row == "/data/1"));
}

#[test]
fn long_header_changes_invalidate_evidence_without_sending_unbounded_credentials() {
    let mut facts = facts();
    facts.fields = vec![FieldFacts {
        location: FieldLocation::RequestHeader,
        path: FieldPath(vec![PathSegment::Key("authorization".into())]),
        ..facts.fields[0].clone()
    }];
    let raw = json!({"key":{"batch_id":"00000000-0000-0000-0000-000000000001","record_id":"00000000-0000-0000-0000-000000000002"},"project_id":facts.summary.project_id,"environment_id":EnvironmentId::new(),"site":null,"source":{"platform":"fixture","platform_version":null,"source_url":null},"observed_at":"2026-10-08T00:00:00Z","context":null,"fact":{"kind":"exchange","request":{"method":"POST","url":"https://fixture.test/api","url_truncated":false,"headers":{"complete":false,"entries":[["Authorization",format!("Bearer {}","synthetic".repeat(100))]]},"body":{"state":"none"}}}});
    let observation = serde_json::from_value(raw.clone()).unwrap();
    facts.requests = vec![
        nexofolio_contracts::request::RequestMaterial::from_observation("first", &observation)
            .unwrap(),
    ];
    let signals = sample_fields(&facts, "first", &raw);
    assert!(
        signals.is_empty(),
        "long values are outside scalar model budget"
    );
    let first = field_inputs(&facts, &signals, &[]);
    assert_eq!(first[0].evidence.len(), 1);
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains(&"synthetic".repeat(100))
    );
    let first_hash = field_input_hash(&facts, None, &first[0]);
    facts.requests[0].example_id = "second-value-evidence".into();
    facts.requests[0].request.headers.as_mut().unwrap().entries[0].1 =
        "changed-long-value".repeat(100);
    let second = field_inputs(&facts, &[], &[]);
    assert_ne!(first_hash, field_input_hash(&facts, None, &second[0]));
    assert_eq!(
        second[0].evidence[0].source.as_ref().unwrap().example,
        "second-value-evidence"
    );
}
