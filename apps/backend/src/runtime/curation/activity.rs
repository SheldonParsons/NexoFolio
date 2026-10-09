//! Safe processing events: no model reasoning, credentials, prompts or sample values.
use chrono::{DateTime, Utc};
use nexofolio_common::EndpointId;
use nexofolio_curate::Pass;
use serde::{Deserialize, Serialize};

pub(super) const HISTORY_LIMIT: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Activity {
    pub id: u64,
    pub at: DateTime<Utc>,
    pub stage: Pass,
    pub kind: String,
    pub message: String,
    pub endpoint: Option<EndpointId>,
    #[serde(default)]
    pub name: Option<String>,
    pub fields: Option<usize>,
    pub batch: Option<usize>,
    pub elapsed_ms: Option<u64>,
}
impl Activity {
    pub(super) fn new(stage: Pass, kind: &str, message: impl Into<String>) -> Self {
        Self {
            id: 0,
            at: Utc::now(),
            stage,
            kind: kind.into(),
            message: message.into(),
            endpoint: None,
            name: None,
            fields: None,
            batch: None,
            elapsed_ms: None,
        }
    }
    pub(super) fn endpoint(mut self, id: EndpointId) -> Self {
        self.endpoint = Some(id);
        self
    }
    pub(super) fn name(mut self, value: Option<String>) -> Self {
        self.name = value;
        self
    }
    pub(super) fn fields(mut self, value: usize) -> Self {
        self.fields = Some(value);
        self
    }
    pub(super) fn batch(mut self, value: usize) -> Self {
        self.batch = Some(value);
        self
    }
    pub(super) fn elapsed(mut self, value: u64) -> Self {
        self.elapsed_ms = Some(value);
        self
    }
}
