#[path = "review_regressions/support.rs"]
#[allow(dead_code)]
mod support;
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::*;
struct Model;
#[async_trait::async_trait]
impl Completions for Model {
    async fn complete(&self, _: &str) -> Result<String, CompletionError> {
        Ok(serde_json::json!({"fields":[{"key":"0","display_name":"模型名","description":"模型基础说明",
            "details":["未提供依据的业务限制"],"unknowns":[]}]}).to_string())
    }
}
#[tokio::test]
async fn direct_name_and_unit_do_not_confirm_model_details() {
    let mut facts = support::facts("/amount");
    facts.fields = vec![support::field("amount")];
    let input = FieldInput {
        observations: None,
        value_examples: vec![],
        key: "request_body:amount".into(),
        path: "amount".into(),
        location: FieldLocation::RequestBody,
        types: vec![ValueType::Number],
        values: vec![],
        parent: "".into(),
        siblings: vec![],
        evidence: vec![FieldEvidence {
            kind: EvidenceKind::DirectSource,
            text: "直接中文标题".into(),
            endpoint: facts.summary.id,
            field: "title".into(),
            example: None,
            environment: Some(EnvironmentId::new()),
            source: None,
            options: vec![FieldOption {
                value: "amount".into(),
                label: Some("金额（万）".into()),
            }],
        }],
    };
    let commands = Describe::new(Model, "fixed".into())
        .fields(&facts, &EndpointKnowledge::default(), &[input])
        .await
        .unwrap();
    let Command::PutFieldNote { context, .. } = &commands[0] else {
        panic!()
    };
    assert_eq!(context.display_name.as_deref(), Some("金额"));
    assert_eq!(context.display_name_confirmation, Confirmation::Direct);
    assert_eq!(context.meaning.unit.as_deref(), Some("万"));
    assert_eq!(context.meaning.unit_confirmation, Confirmation::Direct);
    assert_eq!(context.meaning.confirmation, Confirmation::Candidate);
    assert_eq!(context.meaning.details, ["未提供依据的业务限制"]);
    assert!(context.meaning.unknowns.contains(&"币种未确认".into()));
}
