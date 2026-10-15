//! Read-only pool observations. None means the driver does not expose a counter.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PoolSnapshot {
    pub name: &'static str,
    pub size: u32,
    pub idle: usize,
    pub configured_max: u32,
    pub waiting: Option<u64>,
    pub acquire_timeouts: Option<u64>,
}

pub trait PoolTelemetry: Send + Sync {
    fn pool_snapshots(&self) -> Vec<PoolSnapshot>;
}
