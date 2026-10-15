//! Minimal OpenAI-compatible chat completion client: curate's only IO.
//!
//! The catalog model is served behind an OpenAI-shaped endpoint, so one POST to
//! `/chat/completions` covers every pass. No streaming: a pass wants the whole
//! answer before it writes anything.

use async_trait::async_trait;
use nexofolio_curate_contracts::{CompletionError, Completions};
use serde::{Deserialize, Serialize};
use std::time::Instant;
mod budget;
mod options;
mod telemetry;
use budget::Budget;
use options::Format;
pub use options::{Completion, Measurement, ModelOptions, Thinking};

pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
    slots: Budget,
    options: ModelOptions,
    telemetry: telemetry::Telemetry,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("模型请求配置无效")]
    Configuration,
    #[error("request failed: {0}")]
    Transport(String),
    #[error("模型请求超时")]
    Timeout,
    #[error("模型服务正在冷却")]
    Cooldown { retry_after_seconds: u64 },
    // An upstream error body may echo a prompt or a credential. Never forward it.
    #[error("模型服务返回 HTTP {status}，请检查服务状态后重试")]
    Status {
        status: u16,
        retry_after_seconds: Option<u64>,
    },
    #[error("model returned no choices")]
    Empty,
    #[error("模型服务返回格式不完整，请重试")]
    InvalidResponse,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Format>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Format>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: serde_json::Value,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

impl LlmClient {
    pub fn new(base_url: String, model: String, api_key: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(150))
                .build()
                .expect("curate HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            slots: Budget::new(4),
            options: ModelOptions::default(),
            telemetry: telemetry::Telemetry::default(),
        }
    }
    pub fn with_concurrency(mut self, limit: usize) -> Self {
        self.slots = Budget::new(limit);
        self
    }

    pub fn with_options(mut self, options: ModelOptions) -> Self {
        self.options = options;
        self
    }

    /// One turn, no history. Low temperature: a description should be the same
    /// on a re-run, otherwise a second pass rewrites what the first wrote.
    pub async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        self.complete_measured(prompt).await.map(|r| r.content)
    }

    pub async fn complete_measured(&self, prompt: &str) -> Result<Completion, LlmError> {
        let attempt = self.telemetry.attempt();
        let queued = Instant::now();
        let permit = match self.slots.acquire_ready().await {
            Ok(permit) => permit,
            Err(error) => {
                attempt.finish(queued.elapsed().as_millis() as u64, None, Some(&error));
                return Err(error);
            }
        };
        let queue_ms = queued.elapsed().as_millis() as u64;
        let started = Instant::now();
        let mut result = self.request(prompt).await;
        // Slow generation is a batch retry, not evidence of provider overload.
        let overloaded = matches!(
            &result,
            Err(LlmError::Status {
                status: 429 | 503,
                ..
            })
        );
        let retry_after_seconds = match &result {
            Err(LlmError::Status {
                retry_after_seconds,
                ..
            }) => *retry_after_seconds,
            _ => None,
        };
        permit.observe(
            result.is_ok(),
            overloaded,
            retry_after_seconds.map(std::time::Duration::from_secs),
        );
        if let Ok(reply) = &mut result {
            reply.measurement.queue_ms = queue_ms;
            reply.measurement.elapsed_ms = started.elapsed().as_millis() as u64;
        }
        tracing::info!(
            queue_ms,
            http_status = match &result {
                Err(LlmError::Status { status, .. }) => Some(*status),
                _ => None,
            },
            retry_after_seconds,
            elapsed_ms = started.elapsed().as_millis() as u64,
            input_bytes = prompt.len(),
            output_bytes = result.as_ref().map_or(0, |s| s.content.len()),
            success = result.is_ok(),
            "model request measured"
        );
        attempt.finish(
            queue_ms,
            Some(started.elapsed().as_millis() as u64),
            result.as_ref().err(),
        );
        result
    }

    async fn request(&self, prompt: &str) -> Result<Completion, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
            temperature: 0.2,
            thinking: self.options.thinking.switch(),
            reasoning_effort: self.options.thinking.effort(),
            response_format: self.options.json_output.then_some(Format {
                kind: "json_object",
            }),
        };

        let started = Instant::now();
        let _http = telemetry::Gauge::enter(&self.telemetry.http);
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(transport_error)?;

        let headers_ms = started.elapsed().as_millis() as u64;
        let status = response.status();
        if !status.is_success() {
            return Err(LlmError::Status {
                status: status.as_u16(),
                retry_after_seconds: response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok())
                    .and_then(retry_after_seconds),
            });
        }

        let parsed: ChatResponse = response.json().await.map_err(|e| {
            if e.is_decode() {
                return LlmError::InvalidResponse;
            }
            transport_error(e)
        })?;

        // Optional provider usage is aggregate telemetry only, never prompt/reply content.
        tracing::info!(
            response_headers_ms = headers_ms,
            response_body_ms = (started.elapsed().as_millis() as u64).saturating_sub(headers_ms),
            prompt_tokens = parsed
                .usage
                .get("prompt_tokens")
                .and_then(serde_json::Value::as_u64),
            completion_tokens = parsed
                .usage
                .get("completion_tokens")
                .and_then(serde_json::Value::as_u64),
            reasoning_tokens = parsed
                .usage
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(serde_json::Value::as_u64),
            cached_tokens = parsed
                .usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(serde_json::Value::as_u64),
            "model usage measured"
        );
        let choice = parsed.choices.into_iter().next().ok_or(LlmError::Empty)?;
        if choice
            .finish_reason
            .as_deref()
            .is_some_and(|reason| reason != "stop")
        {
            return Err(LlmError::InvalidResponse);
        }
        if choice.message.content.trim().is_empty() {
            return Err(LlmError::Empty);
        }
        let token = |key: &str| {
            parsed
                .usage
                .pointer(key)
                .and_then(serde_json::Value::as_u64)
        };
        Ok(Completion {
            content: choice.message.content,
            measurement: Measurement {
                prompt_tokens: token("/prompt_tokens"),
                completion_tokens: token("/completion_tokens"),
                reasoning_tokens: token("/completion_tokens_details/reasoning_tokens"),
                cached_tokens: token("/prompt_tokens_details/cached_tokens"),
                response_model: parsed.model,
                finish_reason: choice.finish_reason,
                ..Default::default()
            },
        })
    }
}

fn transport_error(error: reqwest::Error) -> LlmError {
    if error.is_builder() {
        LlmError::Configuration
    } else if error.is_timeout() {
        LlmError::Timeout
    } else {
        LlmError::Transport("模型服务连接失败".into())
    }
}

fn retry_after_seconds(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return std::time::SystemTime::now()
            .checked_add(std::time::Duration::from_secs(seconds))
            .map(|_| seconds);
    }
    httpdate::parse_http_date(value).ok().map(|deadline| {
        deadline
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default()
            .as_secs()
            .saturating_add(1)
    })
}

impl LlmError {
    fn classification(&self) -> CompletionError {
        match self {
            Self::Configuration => CompletionError::Configuration,
            Self::Timeout => CompletionError::Timeout,
            Self::Transport(_) => CompletionError::Network,
            Self::Status {
                status,
                retry_after_seconds,
            } => CompletionError::Status {
                status: *status,
                retry_after_seconds: *retry_after_seconds,
            },
            Self::Cooldown {
                retry_after_seconds,
            } => CompletionError::Cooldown {
                retry_after_seconds: *retry_after_seconds,
            },
            Self::Empty | Self::InvalidResponse => CompletionError::InvalidReply(self.to_string()),
        }
    }
}

#[async_trait]
impl Completions for LlmClient {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        LlmClient::complete(self, prompt)
            .await
            .map_err(|e| e.classification())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinSet,
    };

    #[tokio::test]
    async fn model_budget_limits_all_callers_and_never_requires_a_real_service() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let server_active = active.clone();
        let server_peak = peak.clone();
        let server = tokio::spawn(async move {
            let mut replies = JoinSet::new();
            for _ in 0..8 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let active = server_active.clone();
                let peak = server_peak.clone();
                replies.spawn(async move {
                    let mut request = [0u8; 4096];
                    assert!(stream.read(&mut request).await.unwrap() > 0);
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    let body = r#"{"choices":[{"message":{"content":"fixture"}}]}"#;
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    active.fetch_sub(1, Ordering::SeqCst);
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
            while let Some(result) = replies.join_next().await {
                result.unwrap();
            }
        });
        let client = Arc::new(
            LlmClient::new(
                format!("http://{address}"),
                "fixture".into(),
                "fixture".into(),
            )
            .with_concurrency(2),
        );
        let mut calls = JoinSet::new();
        for _ in 0..8 {
            let client = client.clone();
            calls.spawn(async move { client.complete("fixture").await.unwrap() });
        }
        while let Some(result) = calls.join_next().await {
            assert_eq!(result.unwrap(), "fixture");
        }
        server.await.unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod request_tests;

#[cfg(test)]
mod telemetry_tests;
