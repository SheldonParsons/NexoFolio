//! Public document identity only. This read never grants member or MCP access.
use crate::Environment;
use async_trait::async_trait;
use nexofolio_common::{ProjectId, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DocumentProject {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub environments: Vec<Environment>,
}

#[async_trait]
pub trait ProjectDocuments: Send + Sync {
    /// The instance comes from service configuration, never from the visitor.
    async fn document_project(
        &self,
        instance: &str,
        project: ProjectId,
    ) -> Result<Option<DocumentProject>>;
}
