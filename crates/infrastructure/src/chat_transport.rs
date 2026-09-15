//! Shared bounded HTTP transport for legacy catalog and knowledge maintenance providers.
use nexofolio_contracts::{Error, Result, Secret};
use serde_json::Value;
use std::time::Duration;
pub(crate) struct ChatTransport {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    key: Secret,
}
fn invalid() -> Error {
    Error::InvalidInput {
        message: "invalid model endpoint configuration".into(),
    }
}
impl ChatTransport {
    pub(crate) fn new(base: &str, key: Secret, timeout: Duration) -> Result<Self> {
        let endpoint =
            reqwest::Url::parse(&format!("{}/chat/completions", base.trim_end_matches('/')))
                .map_err(|_| invalid())?;
        if !["http", "https"].contains(&endpoint.scheme())
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || key.expose().trim().is_empty()
        {
            return Err(invalid());
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            endpoint,
            key,
        })
    }
    pub(crate) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
    pub(crate) async fn complete(&self, request: &Value, max_bytes: usize) -> Result<Value> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.key.expose())
            .json(request)
            .send()
            .await
            .map_err(|e| Error::Unavailable {
                component: if e.is_timeout() {
                    "maintenance_model_timeout"
                } else {
                    "maintenance_model_transport"
                },
            })?;
        if !response.status().is_success() {
            let message = match response.status().as_u16() {
                401 | 403 => "MODEL_AUTHENTICATION_FAILED",
                429 => "MODEL_RATE_LIMITED",
                500..=599 => "MODEL_PROVIDER_FAILED",
                _ => "MODEL_REQUEST_REJECTED",
            };
            return Err(Error::InvalidInput {
                message: message.into(),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable {
            component: "model_response",
        })? {
            if bytes.len() + chunk.len() > max_bytes {
                return Err(Error::InvalidInput {
                    message: "MODEL_RESPONSE_TOO_LARGE".into(),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidInput {
            message: "MODEL_INVALID_RESPONSE".into(),
        })
    }
}
