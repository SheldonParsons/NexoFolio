//! Read-only compatibility for persisted historical directory candidates.
use async_trait::async_trait;
use nexofolio_contracts::{JobId, ProjectId, Result};

/// Authenticated project readers are separate from privileged administrative commands.
#[async_trait]
pub trait CatalogPreviewReader: Send + Sync {
    async fn list_previews(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
        status: Option<nexofolio_contracts::PreviewStatus>,
    ) -> Result<nexofolio_contracts::CatalogPreviewPage>;
    async fn preview_detail(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        task: JobId,
    ) -> Result<nexofolio_contracts::CatalogPreviewDetail>;
}
