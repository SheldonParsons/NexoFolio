//! Project-wide write barrier. The serving application wires the implementation.
use async_trait::async_trait;
use nexofolio_common::{ProjectId, Result};

pub const PROJECT_CONTENT_LOCK: i32 = 0x7072_0001;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ProjectContentCounts {
    pub interfaces: u64,
    pub structures: u64,
    pub folders: u64,
    pub curation_results: u64,
    pub running_curations: u64,
}

/// Dropping the guard releases the activity lease, including cancellation paths.
pub trait ProjectActivity: Send {}

#[async_trait]
pub trait ProjectLifecycle: Send + Sync {
    async fn activity(&self, project: ProjectId) -> Result<Box<dyn ProjectActivity>>;
}
