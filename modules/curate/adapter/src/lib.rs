//! Minimal OpenAI-compatible chat completion client: curate's only IO.
//!
//! The catalog model is served behind an OpenAI-shaped endpoint, so one POST to
//! `/chat/completions` covers every pass. No streaming: a pass wants the whole
//! answer before it writes anything.

use async_trait::async_trait;
use nexofolio_curate_contracts::{CompletionError, Completions};
use serde::{Deserialize, Serialize};

pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("request failed: {0}")]
    Transport(String),
    #[error("model returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("model returned no choices")]
    Empty,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

impl LlmClient {
    pub fn new(base_url: String, model: String, api_key: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
        }
    }

    /// One turn, no history. Low temperature: a description should be the same
    /// on a re-run, otherwise a second pass rewrites what the first wrote.
    pub async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
            temperature: 0.2,
        };

        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Status {
                status: status.as_u16(),
                body,
            });
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or(LlmError::Empty)
    }
}

#[async_trait]
impl Completions for LlmClient {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        LlmClient::complete(self, prompt)
            .await
            .map_err(|e| CompletionError(e.to_string()))
    }
}
