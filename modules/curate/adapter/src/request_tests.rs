use super::*;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
async fn reply(value: Value) -> (LlmClient, tokio::task::JoinHandle<Value>) {
    reply_status(value, "200 OK", "").await
}
async fn reply_status(
    value: Value,
    status: &'static str,
    extra: &'static str,
) -> (LlmClient, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let (body_start, body_len) = loop {
            let mut part = [0; 4096];
            let n = stream.read(&mut part).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&part[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let len: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                break (end + 4, len);
            }
        };
        while bytes.len() < body_start + body_len {
            let mut part = [0; 4096];
            let n = stream.read(&mut part).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&part[..n]);
        }
        let request = serde_json::from_slice(&bytes[body_start..body_start + body_len]).unwrap();
        let body = value.to_string();
        let response = format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        request
    });
    (
        LlmClient::new(
            format!("http://{address}"),
            "fixture".into(),
            "fixture".into(),
        ),
        server,
    )
}
#[tokio::test]
async fn explicit_controls_reach_gateway_and_only_final_content_is_used() {
    for thinking in [
        Thinking::Provider,
        Thinking::Low,
        Thinking::High,
        Thinking::Max,
        Thinking::Disabled,
    ] {
        let (client, server) = reply(json!({"model":"fixture-v1","choices":[{"finish_reason":"stop","message":{"content":"{}","reasoning_content":"not stored"}}],"usage":{"prompt_tokens":3,"completion_tokens":8,"completion_tokens_details":{"reasoning_tokens":5}}})).await;
        let result = client
            .with_options(ModelOptions {
                thinking,
                json_output: true,
            })
            .complete_measured("Return JSON")
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert_eq!(request["response_format"]["type"], "json_object");
        match thinking {
            Thinking::Provider => {
                assert!(request.get("thinking").is_none());
                assert!(request.get("reasoning_effort").is_none());
            }
            Thinking::Low | Thinking::High | Thinking::Max => {
                assert_eq!(request["thinking"]["type"], "enabled");
                assert_eq!(request["reasoning_effort"], thinking.effort().unwrap());
            }
            Thinking::Disabled => {
                assert_eq!(request["thinking"]["type"], "disabled");
                assert!(request.get("reasoning_effort").is_none());
            }
        }
        assert_eq!(result.content, "{}");
        assert_eq!(result.measurement.reasoning_tokens, Some(5));
        assert_eq!(
            result.measurement.response_model.as_deref(),
            Some("fixture-v1")
        );
        assert!(
            !serde_json::to_string(&result.measurement)
                .unwrap()
                .contains("not stored")
        );
    }
}
#[tokio::test]
async fn truncated_or_aborted_json_and_empty_content_are_not_accepted() {
    for reason in ["length", "content_filter", "insufficient_system_resource"] {
        let (client, server) =
            reply(json!({"choices":[{"finish_reason":reason,"message":{"content":"{}"}}]})).await;
        assert!(matches!(
            client.complete("fixture").await,
            Err(LlmError::InvalidResponse)
        ));
        server.await.unwrap();
    }
    let (client, server) =
        reply(json!({"choices":[{"finish_reason":"stop","message":{"content":" "}}]})).await;
    assert!(matches!(
        client.complete("fixture").await,
        Err(LlmError::Empty)
    ));
    server.await.unwrap();
}

#[test]
fn retry_after_is_parsed_without_reading_an_upstream_error_body() {
    assert_eq!(retry_after_seconds("65"), Some(65));
    assert_eq!(retry_after_seconds("172800"), Some(172800));
    assert_eq!(retry_after_seconds("not a delay"), None);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(90);
    let seconds = retry_after_seconds(&httpdate::fmt_http_date(later)).unwrap();
    assert!((89..=91).contains(&seconds));
    assert_eq!(retry_after_seconds("18446744073709551615"), None);
}

#[tokio::test]
async fn upstream_rate_limit_sets_shared_cooldown_without_exposing_response_body() {
    let (client, server) = reply_status(
        json!({"error":"private upstream body"}),
        "429 Too Many Requests",
        "Retry-After: 65\r\n",
    )
    .await;
    let error = match client.complete("fixture").await {
        Err(e) => e,
        Ok(_) => panic!("429 must fail this attempt"),
    };
    assert!(matches!(
        error,
        LlmError::Status {
            status: 429,
            retry_after_seconds: Some(65)
        }
    ));
    assert!(!error.to_string().contains("private upstream body"));
    server.await.unwrap();
    assert!(
        matches!(
            client.complete("next caller").await,
            Err(LlmError::Cooldown {
                retry_after_seconds: 64..=66
            })
        ),
        "cooldown is returned to the durable queue without HTTP IO or holding a permit"
    );
}

#[tokio::test]
async fn invalid_request_configuration_is_not_a_temporary_connection_failure() {
    let client = LlmClient::new(
        "invalid://fixture".into(),
        "fixture".into(),
        "fixture".into(),
    );
    let error = Completions::complete(&client, "fixture").await.unwrap_err();
    assert_eq!(error.code(), "model_configuration");
    assert!(!error.temporary());
}
