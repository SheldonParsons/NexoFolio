use async_trait::async_trait;
use nexofolio_contracts::{
    DirectoryCandidate, GeneratorInfo, JobId, PreviewReview, PreviewTask, ProjectId, Result,
};

/// Administrative-only preview storage. No method here can publish or change interface definitions.
#[async_trait]
pub trait CatalogPreviewStore: Send + Sync {
    async fn create(&self, project: ProjectId) -> Result<PreviewTask>;
    async fn read(&self, task: JobId) -> Result<PreviewTask>;
    /// Claims pending, failed, or expired running tasks with a new fenced generation.
    async fn claim(&self, task: JobId, generator: &GeneratorInfo) -> Result<PreviewTask>;
    async fn complete(
        &self,
        task: &PreviewTask,
        candidate: &DirectoryCandidate,
        review: &PreviewReview,
    ) -> Result<()>;
    async fn fail(&self, task: &PreviewTask, code: &'static str) -> Result<()>;
}

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
