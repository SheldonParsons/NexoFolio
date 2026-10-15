use super::Status;
use super::tests::*;
use async_trait::async_trait;
use nexofolio_contracts::endpoint::EndpointFacts;
use nexofolio_contracts::endpoint::{FieldFacts, FieldLocation, FieldPath, PathSegment, ValueType};
use nexofolio_curate::{CompletionError, Completions, Describe};
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct BatchModel {
    inner: Arc<Model>,
    fail: AtomicBool,
    calls: Mutex<Vec<Vec<String>>>,
}
#[async_trait]
impl Completions for BatchModel {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        if !prompt.contains("字段和可核查的证据：") {
            return self.inner.complete(prompt).await;
        }
        let raw = prompt
            .split("字段和可核查的证据：")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let fields: Vec<Value> = serde_json::from_str(raw).unwrap();
        self.calls.lock().unwrap().push(
            fields
                .iter()
                .map(|f| f["path"].as_str().unwrap().into())
                .collect(),
        );
        if fields.iter().any(|f| f["path"] == "f24") && self.fail.load(Ordering::SeqCst) {
            return Err(CompletionError::InvalidReply("模型服务响应超时".into()));
        }
        Ok(
            serde_json::json!({"fields": fields.iter().map(|f| serde_json::json!({
            "key":f["key"], "display_name":"合成字段", "description":"业务字段，具体含义待确认"
        })).collect::<Vec<_>>()})
            .to_string(),
        )
    }
}

#[tokio::test]
async fn late_batch_failure_keeps_acknowledged_chunks_and_retry_only_requests_missing_fields() {
    let (mut s, project, memory, model) = fixture(false);
    let mut facts = Vec::new();
    for summary in s.endpoints.list(project).await.unwrap() {
        let fields = if summary.path_template == "/api/1" {
            (0..49)
                .map(|i| FieldFacts {
                    location: FieldLocation::ResponseBody { status: 200 },
                    path: FieldPath(vec![PathSegment::Key(format!("f{i:02}"))]),
                    types: vec![ValueType::String],
                    labels: vec![],
                    differs_between_environments: false,
                    declared: None,
                    conflict: None,
                })
                .collect()
        } else {
            vec![]
        };
        facts.push(EndpointFacts {
            requests: vec![],
            summary,
            fields,
            aliases: vec![],
            addresses: vec![],
            examples: vec![],
        });
    }
    s.endpoints = Arc::new(Endpoints(facts));
    let batch_model = Arc::new(BatchModel {
        inner: model,
        fail: AtomicBool::new(true),
        calls: Mutex::default(),
    });
    s.describe = Arc::new(Describe::new(
        batch_model.clone() as Arc<dyn Completions>,
        "test".into(),
    ));
    let failed = execute(&s, project).await;
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(
        (
            failed.fields_total,
            failed.fields_updated,
            failed.fields_failed
        ),
        (49, 48, 1)
    );
    assert_eq!(failed.issues[0].code, "invalid_reply");
    assert_eq!(failed.issues[0].fields, 1);
    assert!(failed.retries >= 3);
    assert_eq!(failed.recovery_sweeps, 0);
    assert!(failed.active.is_empty());
    assert_eq!(memory.data.lock().unwrap().fields.len(), 48);
    assert_eq!(memory.data.lock().unwrap().directory_writes, 2);
    batch_model.calls.lock().unwrap().clear();
    batch_model.fail.store(false, Ordering::SeqCst);
    let retry = execute(&s, project).await;
    assert_eq!(retry.status, Status::Done);
    assert_eq!((retry.fields_total, retry.fields_updated), (1, 1));
    let calls = batch_model.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].len(), 1);
    assert!(calls[0].iter().all(|p| p == "f24"));
    assert_eq!(memory.data.lock().unwrap().fields.len(), 49);
}

#[test]
fn upstream_error_details_are_classified_without_forwarding_content() {
    assert_eq!(
        nexofolio_curate::CompletionError::Status {
            status: 401,
            retry_after_seconds: None
        }
        .code(),
        "model_rejected"
    );
    assert_eq!(
        nexofolio_curate::CompletionError::InvalidReply("字段回复含未知或重复 key".into()).code(),
        "invalid_reply"
    );
    assert_eq!(
        nexofolio_curate::CompletionError::Timeout.code(),
        "model_timeout"
    );
}
