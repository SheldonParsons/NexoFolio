use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::{
    CompletionError, Completions, Describe, field_addressed, field_input_hash, field_inputs,
    pending_fields,
};
use serde_json::json;
struct Reply(String);
#[async_trait]
impl Completions for Reply {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        assert!(prompt.contains("requested"));
        Ok(self.0.clone())
    }
}
fn facts() -> EndpointFacts {
    EndpointFacts {
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: ProjectId::new(),
            method: "POST".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        requests: vec![],
        examples: vec![],
        fields: vec![FieldFacts {
            location: FieldLocation::RequestHeader,
            path: FieldPath(vec![PathSegment::Key("x-type".into())]),
            types: vec![ValueType::String],
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        }],
    }
}
fn note(command: Command) -> FieldNote {
    let Command::PutFieldNote {
        endpoint,
        path,
        text,
        values,
        context,
    } = command
    else {
        panic!()
    };
    FieldNote {
        endpoint,
        path,
        text,
        values,
        context,
        author: Author::Curate {
            pass: "fields".into(),
            model: None,
        },
        at: "1970-01-01T00:00:00Z".parse().unwrap(),
    }
}
#[tokio::test]
async fn missing_name_is_saved_as_partial_then_only_name_is_requested_and_good_description_survives()
 {
    let facts = facts();
    let inputs = field_inputs(&facts, &[], &[]);
    let mut knowledge = EndpointKnowledge::default();
    let model = Describe::new(
        Reply(json!({"fields":[{"key":"0","description":"区分调用来源"}]}).to_string()),
        "fixed".into(),
    );
    let first = model
        .fields_batch_partial(&facts, &knowledge, &inputs)
        .await
        .unwrap();
    assert_eq!(first.missing.len(), 1);
    assert_eq!(first.commands.len(), 1);
    let first = note(first.commands.into_iter().next().unwrap());
    assert!(!field_addressed(&first));
    assert_eq!(
        first.context.completion.as_ref().unwrap().name.state,
        AspectState::Missing
    );
    knowledge.fields.push(first);
    struct NameOnly;
    #[async_trait]
    impl Completions for NameOnly {
        async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
            assert!(prompt.contains("\"requested\":[\"display_name\"]"));
            Ok(json!({"fields":[{"key":"0","display_name":"来源类型","description":"不能覆盖已保存的说明"}]}).to_string())
        }
    }
    let second = Describe::new(NameOnly, "fixed".into())
        .fields_batch_partial(&facts, &knowledge, &inputs)
        .await
        .unwrap();
    assert!(second.missing.is_empty());
    let saved = note(second.commands.into_iter().next().unwrap());
    assert!(field_addressed(&saved));
    assert_eq!(saved.text, "区分调用来源");
    assert_eq!(saved.context.display_name.as_deref(), Some("来源类型"));
    knowledge.fields = vec![saved];
    assert!(pending_fields(&facts, &knowledge, inputs).is_empty());
}
#[tokio::test]
async fn explicit_uncertainty_is_reviewable_without_fabricating_a_name_or_infinite_retries() {
    let facts = facts();
    let inputs = field_inputs(&facts, &[], &[]);
    let model=Describe::new(Reply(json!({"fields":[{"key":"0","description":"页面设置的自定义请求头","name_unknown":"样本没有提供这个缩写的业务含义"}]}).to_string()),"fixed".into());
    let out = model
        .fields_batch_partial(&facts, &EndpointKnowledge::default(), &inputs)
        .await
        .unwrap();
    assert!(out.missing.is_empty());
    let saved = note(out.commands.into_iter().next().unwrap());
    assert!(saved.context.display_name.is_none());
    assert!(field_addressed(&saved));
    assert!(saved.context.completion.as_ref().unwrap().uncertain());
}
#[tokio::test]
async fn blank_description_or_placeholder_name_never_counts_as_complete() {
    let facts = facts();
    let inputs = field_inputs(&facts, &[], &[]);
    for reply in [
        json!({"fields":[{"key":"0","display_name":"来源类型"}]}),
        json!({"fields":[{"key":"0","display_name":"待确认","description":"来源类型编码"}]}),
    ] {
        let out = Describe::new(Reply(reply.to_string()), "fixed".into())
            .fields_batch_partial(&facts, &EndpointKnowledge::default(), &inputs)
            .await
            .unwrap();
        assert_eq!(out.missing.len(), 1);
    }
}
#[test]
fn a_matching_hash_without_a_name_is_still_pending_but_manual_notes_stay_protected() {
    let facts = facts();
    let input = field_inputs(&facts, &[], &[]);
    let mut n = FieldNote {
        endpoint: facts.summary.id,
        path: input[0].key.clone(),
        text: "已保存说明".into(),
        values: vec![],
        context: FieldContext {
            input_hash: Some(field_input_hash(&facts, None, &input[0])),
            ..Default::default()
        },
        author: Author::Curate {
            pass: "fields".into(),
            model: None,
        },
        at: "1970-01-01T00:00:00Z".parse().unwrap(),
    };
    let k = EndpointKnowledge {
        fields: vec![n.clone()],
        ..Default::default()
    };
    assert_eq!(pending_fields(&facts, &k, input.clone()).len(), 1);
    n.author = Author::Person {
        user: nexofolio_common::UserId::new(),
    };
    let k = EndpointKnowledge {
        fields: vec![n],
        ..Default::default()
    };
    assert!(pending_fields(&facts, &k, input).is_empty());
}
