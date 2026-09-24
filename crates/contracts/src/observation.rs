//! Source-independent observations handed from intake to observe.
//!
//! Intake turns every accepted collect record into a [`CanonicalObservation`]:
//! wire details (base64, header states, record envelopes) are gone, and the
//! target has already been resolved. Observe never sees the wire format.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EnvironmentId, ProjectId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::scope::SiteScope;

/// Stable identity of one collected record: retries of the same batch produce
/// the same key, so sinks can ignore duplicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObservationKey {
    pub batch_id: Uuid,
    pub record_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalObservation {
    pub key: ObservationKey,
    pub project_id: ProjectId,
    /// Always set for exchanges. `None` means a declaration that applies to
    /// every environment of the project.
    pub environment_id: Option<EnvironmentId>,
    pub site: Option<SiteScope>,
    pub source: Source,
    pub observed_at: DateTime<Utc>,
    pub fact: Fact,
    /// Browser context, stored as-is. Nothing may depend on it being present.
    pub context: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub platform: String,
    pub platform_version: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    Exchange(HttpExchange),
    Declaration(HttpDeclaration),
}

/// A call that actually happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpExchange {
    pub request: HttpRequest,
    /// `None` when no response was captured.
    pub response: Option<HttpResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    /// Uppercase, e.g. `GET`.
    pub method: String,
    /// Absolute URL including the query string.
    pub url: String,
    pub url_truncated: bool,
    pub headers: Option<Headers>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Option<Headers>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Headers {
    /// `false` when the client could only see some of the headers.
    pub complete: bool,
    /// Names as sent, in order; duplicates allowed.
    pub entries: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Body {
    /// The whole body. Only this state proves that a field is absent.
    Full {
        media_type: Option<String>,
        content: Content,
    },
    /// A trustworthy beginning of the body. May add structure, never removes it.
    Truncated {
        media_type: Option<String>,
        content: Content,
        original_bytes: Option<u64>,
    },
    /// There was a body but it could not be read.
    Unreadable {
        media_type: Option<String>,
        note: Option<String>,
    },
    /// There was no body.
    None,
}

impl Body {
    /// Whether a missing field in this body is evidence that the field is absent.
    pub fn proves_absence(&self) -> bool {
        matches!(self, Self::Full { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum Content {
    Text(String),
    /// Already decoded from base64 by intake.
    Binary(Vec<u8>),
}

/// A declared endpoint shape (Swagger/OpenAPI sync or manual entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpDeclaration {
    pub method: String,
    /// Template with `{name}` placeholders, given by the declaring source.
    pub path_template: String,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub request: DeclaredRequest,
    /// Keyed by `200`, `4XX`, `default`, …
    pub responses: BTreeMap<String, DeclaredResponse>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclaredRequest {
    pub path_params: BTreeMap<String, DeclaredParameter>,
    pub query: BTreeMap<String, DeclaredParameter>,
    pub headers: BTreeMap<String, DeclaredParameter>,
    pub body: Option<DeclaredBody>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclaredParameter {
    pub required: bool,
    pub description: Option<String>,
    /// JSON Schema with every `$ref` already resolved.
    pub schema: Option<Value>,
    pub example: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclaredBody {
    pub required: bool,
    pub media_type: Option<String>,
    pub schema: Option<Value>,
    pub example: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclaredResponse {
    pub description: Option<String>,
    pub media_type: Option<String>,
    pub schema: Option<Value>,
    pub example: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SinkError {
    /// Retryable: the whole batch should be retried with the same keys.
    #[error("observation sink unavailable")]
    Unavailable,
}

/// Where intake delivers observations. Implemented by observe.
#[async_trait]
pub trait ObservationSink: Send + Sync {
    /// Accepts every observation or fails as a whole. Delivering a key that was
    /// already accepted must not change any count, so intake can retry freely.
    async fn accept(&self, observations: Vec<CanonicalObservation>) -> Result<(), SinkError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_full_bodies_prove_absence() {
        let text = || Content::Text("{}".into());
        assert!(
            Body::Full {
                media_type: None,
                content: text()
            }
            .proves_absence()
        );
        let truncated = Body::Truncated {
            media_type: None,
            content: text(),
            original_bytes: None,
        };
        assert!(!truncated.proves_absence());
        assert!(
            !Body::Unreadable {
                media_type: None,
                note: None
            }
            .proves_absence()
        );
        assert!(!Body::None.proves_absence());
    }

    #[test]
    fn body_serialises_with_the_contract_state_names() {
        let body = Body::Full {
            media_type: Some("application/json".into()),
            content: Content::Text("{}".into()),
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["state"], "full");
        assert_eq!(value["content"]["encoding"], "text");
        assert_eq!(serde_json::from_value::<Body>(value).unwrap(), body);
        assert_eq!(serde_json::to_value(Body::None).unwrap()["state"], "none");
    }
}
