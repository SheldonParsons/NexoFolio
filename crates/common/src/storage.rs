//! Bounded, module-owned storage observations. Null sizes are unknown, never zero.
use crate::{ProjectId, Result};
use async_trait::async_trait;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct StorageGroup {
    pub name: String,
    pub rows: Option<i64>,
    pub column_bytes: Option<i64>,
    pub logical_bytes: Option<i64>,
}
#[derive(Clone, Serialize)]
pub struct StorageTable {
    pub name: String,
    pub table_bytes: i64,
    pub index_bytes: i64,
}
#[derive(Clone, Serialize)]
pub struct StorageUsage {
    pub groups: Vec<StorageGroup>,
    /// Whole shared tables, including TOAST. Never attributable to a project.
    pub tables: Vec<StorageTable>,
}
#[async_trait]
pub trait StorageReader: Send + Sync {
    async fn storage_usage(&self, project: ProjectId) -> Result<StorageUsage>;
}
