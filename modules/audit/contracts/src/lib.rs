mod access;
pub use access::AccessQuery;
// Bounded operational evidence. No credentials, bodies, query strings or free-form messages.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Execution {
    pub task_id: uuid::Uuid,
    pub run_id: Option<uuid::Uuid>,
    pub generation: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Execution>,
    pub at: DateTime<Utc>,
    pub kind: String,
    pub action: String,
    pub actor: String,
    pub route: String,
    pub method: String,
    pub status: u16,
    pub duration_ms: u32,
    pub request_id: String,
    pub project: String,
    pub resource: String,
    pub code: String,
}
impl Event {
    /// Validate at the persistence boundary as well as at the transport boundary.
    pub fn bounded(&self) -> bool {
        self.execution.as_ref().is_none_or(|e| e.generation >= 0)
            && [&self.kind, &self.action, &self.method, &self.code]
                .iter()
                .all(|s| s.len() <= 64)
            && self.actor.len() <= 64
            && self.route.len() <= 200
            && [&self.request_id, &self.project, &self.resource]
                .iter()
                .all(|s| s.len() <= 36)
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub events: u32,
    pub buckets: u32,
    pub operation_days: u32,
    pub error_days: u32,
    pub access_days: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            events: 100_000,
            buckets: 100_000,
            operation_days: 90,
            error_days: 30,
            access_days: 30,
        }
    }
}

#[derive(Default)]
pub struct Filter {
    pub route: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub actor: Option<String>,
    pub request_id: Option<String>,
    pub project: Option<String>,
    pub kind: Option<String>,
    pub limit: u32,
}

#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Atomic batch, hourly upserts for access, individual rows for operations and problems.
    async fn write(&self, events: &[Event], limits: &Limits) -> Result<()>;
    async fn purge(&self, limits: &Limits) -> Result<()>;
    async fn report(&self, filter: &Filter) -> Result<serde_json::Value>;
    async fn access_report(&self, _query: &AccessQuery) -> Result<serde_json::Value> {
        Err(nexofolio_common::Error::Unavailable {
            component: "audit_access",
        })
    }
    /// Exact project AND candidate correlation, never a project-wide log merge.
    async fn curation_events(
        &self,
        _project: &str,
        _candidate: Option<&str>,
        _task: Option<&str>,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!({"items":[],"total":0,"available":false}))
    }
}
