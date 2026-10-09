use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::{
    CompletionError, Completions, Describe, FIELD_BATCH_BYTES, field_batches, field_inputs,
};
use serde_json::json;

fn facts() -> EndpointFacts {
    EndpointFacts {
        requests: vec![],
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: ProjectId::new(),
            method: "GET".into(),
            path_template: "/fixture".into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        fields: (0..80)
            .map(|i| FieldFacts {
                location: FieldLocation::ResponseBody { status: 200 },
                path: FieldPath(vec![PathSegment::Key(format!("f{i}"))]),
                types: vec![ValueType::String],
                labels: vec![],
                differs_between_environments: false,
                declared: None,
                conflict: None,
            })
            .collect(),
        aliases: vec![],
        addresses: vec![],
        examples: vec![],
    }
}
#[test]
fn input_budget_preserves_every_field_and_all_evidence_even_for_oversized_single_fields() {
    let facts = facts();
    let mut inputs = field_inputs(&facts, &[], &[]);
    for input in &mut inputs {
        input.values = vec!["large fixture".repeat(250)];
    }
    inputs[0].values = vec!["x".repeat(FIELD_BATCH_BYTES * 2)];
    let expected: Vec<_> = inputs
        .iter()
        .map(|i| (i.key.clone(), i.values.clone(), i.siblings.clone()))
        .collect();
    let batches = field_batches(inputs, &EndpointKnowledge::default());
    assert_eq!(batches[0].len(), 1);
    assert!(batches.len() > 80usize.div_ceil(24));
    assert!(batches.iter().all(|b| !b.is_empty() && b.len() <= 24));
    let actual: Vec<_> = batches
        .into_iter()
        .flatten()
        .map(|i| (i.key, i.values, i.siblings))
        .collect();
    assert_eq!(actual, expected);
}
struct Reply(String);
#[async_trait]
impl Completions for Reply {
    async fn complete(&self, _: &str) -> Result<String, CompletionError> {
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn missing_entries_preserve_valid_outputs_but_unknown_or_aliased_duplicate_keys_reject_all() {
    let facts = facts();
    let inputs = field_inputs(&facts, &[], &[]);
    let good =
        json!({"fields":[{"key":"0","display_name":"合成字段","description":"有依据的基础解释"}]})
            .to_string();
    let result = Describe::new(Reply(good), "fixture".into())
        .fields_batch_partial(&facts, &EndpointKnowledge::default(), &inputs[..2])
        .await
        .unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(result.missing.len(), 1);
    assert_eq!(result.missing[0].key, inputs[1].key);
    for key in ["unknown".to_string(), inputs[0].key.clone()] {
        let reply = json!({"fields":[{"key":"0","display_name":"合成字段","description":"有效"},{"key":key,"description":"无效或重复"}]}).to_string();
        assert!(
            Describe::new(Reply(reply), "fixture".into())
                .fields_batch_partial(&facts, &EndpointKnowledge::default(), &inputs[..2])
                .await
                .is_err()
        );
    }
}
