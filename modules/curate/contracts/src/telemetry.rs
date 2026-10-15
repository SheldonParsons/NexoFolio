//! Instance-local observations, never provider admission decisions.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Latency {
    pub samples: usize,
    pub mean_ms: Option<f64>,
    pub p95_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelSnapshot {
    pub http_in_flight: usize,
    pub permits_in_use: usize,
    pub waiting_for_permit: usize,
    pub current_limit: usize,
    pub configured_limit: usize,
    pub cooldown_remaining_ms: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub failures: BTreeMap<String, u64>,
    /// Most recent 256 completed attempts, not an unbounded per-request history.
    pub queue: Latency,
    pub request: Latency,
}
pub trait ModelTelemetry: Send + Sync {
    fn model_snapshot(&self) -> Option<ModelSnapshot>;
}
