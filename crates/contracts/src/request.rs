//! Shared captured HTTP material. Values are internal product data, never log data.
use crate::observation::{Body, CanonicalObservation, Content, Fact, Headers, HttpRequest, Source};
use chrono::{DateTime, Utc};
use nexofolio_common::EnvironmentId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationInput {
    pub carrier: String,
    pub scheme: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub source_example: String,
    /// Observing a credential neither proves requiredness nor current validity.
    pub current_validity: String,
    pub required: Option<bool>,
    pub acquisition_source: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestMaterial {
    pub example_id: String,
    #[serde(default)]
    pub observation_key: Option<crate::observation::ObservationKey>,
    pub environment_id: Option<EnvironmentId>,
    pub observed_at: DateTime<Utc>,
    pub source: Source,
    pub request: HttpRequest,
    /// Parsed JSON root, including {}, [], null and scalar values. Body.state stays authoritative.
    pub body_root: Option<Value>,
    pub body_root_kind: Option<crate::endpoint::ValueType>,
    pub response_status: Option<u16>,
    pub response_headers: Option<Headers>,
    pub authentication: Vec<AuthenticationInput>,
    pub gaps: Vec<String>,
}
impl RequestMaterial {
    pub fn from_observation(example: &str, observation: &CanonicalObservation) -> Option<Self> {
        let Fact::Exchange(exchange) = &observation.fact else {
            return None;
        };
        let request = &exchange.request;
        let mut gaps = Vec::new();
        if request.headers.as_ref().is_none_or(|h| !h.complete) {
            gaps.push("request_headers_incomplete: only captured headers are known; browser-added headers may be missing".into());
        }
        if request.url_truncated {
            gaps.push("request_url_truncated".into());
        }
        match request.body {
            Body::Partial { .. } => gaps
                .push("body_summary_only: supply original content or file bytes separately".into()),
            Body::Unreadable { .. } => gaps.push("body_unreadable: recapture required".into()),
            Body::Truncated { .. } => gaps.push("body_truncated: recapture required".into()),
            _ => {}
        }
        let authentication: Vec<_> = request
            .headers
            .iter()
            .flat_map(|h| &h.entries)
            .filter(|(name, _)| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "authorization"
                        | "proxy-authorization"
                        | "cookie"
                        | "x-api-key"
                        | "api-key"
                        | "x-auth-token"
                )
            })
            .map(|(name, value)| AuthenticationInput {
                carrier: name.clone(),
                scheme: name.eq_ignore_ascii_case("authorization").then(|| {
                    match value
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "bearer" => "Bearer",
                        "basic" => "Basic",
                        "digest" => "Digest",
                        _ => "Unknown",
                    }
                    .to_owned()
                }),
                observed_at: observation.observed_at,
                source_example: example.into(),
                current_validity: "unknown_historical_observation".into(),
                required: None,
                acquisition_source: None,
            })
            .collect();
        gaps.push("current_business_authentication_unverified: obtain current credentials from the caller; NexoFolio MCP access token is unrelated".into());
        Some(Self {
            example_id: example.into(),
            observation_key: Some(observation.key),
            environment_id: observation.environment_id,
            observed_at: observation.observed_at,
            source: observation.source.clone(),
            request: request.clone(),
            body_root: body_root(&request.body),
            body_root_kind: body_root(&request.body).map(|v| match v {
                Value::Null => crate::endpoint::ValueType::Null,
                Value::Bool(_) => crate::endpoint::ValueType::Boolean,
                Value::Number(_) => crate::endpoint::ValueType::Number,
                Value::String(_) => crate::endpoint::ValueType::String,
                Value::Array(_) => crate::endpoint::ValueType::Array,
                Value::Object(_) => crate::endpoint::ValueType::Object,
            }),
            response_status: exchange.response.as_ref().map(|r| r.status),
            response_headers: exchange.response.as_ref().and_then(|r| r.headers.clone()),
            authentication,
            gaps,
        })
    }
}
pub fn body_root(body: &Body) -> Option<Value> {
    let content = match body {
        Body::Full { content, .. }
        | Body::Truncated { content, .. }
        | Body::Partial { content, .. } => content,
        _ => return None,
    };
    match content {
        Content::Text(text) => serde_json::from_str(text).ok(),
        Content::Binary(_) => None,
    }
}
