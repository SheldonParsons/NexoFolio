use async_trait::async_trait;
use nexofolio_contracts::*;
use nexofolio_knowledge::CatalogPreviewReader;
use nexofolio_rebuild::{DirectoryReviewer, StructuralDirectoryReviewer};
use std::sync::Arc;

#[async_trait]
pub trait CatalogActivationStore: Send + Sync {
    async fn publish(
        &self,
        user: UserId,
        project: ProjectId,
        request: &PublishCatalog,
        validated: &PreviewTask,
    ) -> Result<CatalogActivation>;
    async fn restore(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreCatalog,
    ) -> Result<CatalogActivation>;
}
pub struct CatalogPublicationService {
    previews: Arc<dyn CatalogPreviewReader>,
    store: Arc<dyn CatalogActivationStore>,
    reviewer: Arc<dyn DirectoryReviewer>,
}
impl CatalogPublicationService {
    pub fn new(
        previews: Arc<dyn CatalogPreviewReader>,
        store: Arc<dyn CatalogActivationStore>,
    ) -> Self {
        Self {
            previews,
            store,
            reviewer: Arc::new(StructuralDirectoryReviewer),
        }
    }
    pub async fn publish(
        &self,
        user: UserId,
        project: ProjectId,
        request: &PublishCatalog,
    ) -> Result<CatalogActivation> {
        if request.expected_generation < 0 {
            return Err(Error::invalid("invalid directory generation"));
        }
        let task = self
            .previews
            .preview_detail(user, project, request.task_id)
            .await?
            .task;
        if !matches!(task.status, PreviewStatus::Ready) {
            return Err(Error::invalid("candidate is not ready"));
        }
        let candidate = task.candidate.as_ref().ok_or(Error::Conflict)?;
        if !self
            .reviewer
            .review(&task.snapshot, candidate)
            .structurally_valid
        {
            return Err(Error::invalid("candidate does not pass publication checks"));
        }
        self.store.publish(user, project, request, &task).await
    }
    pub async fn restore(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreCatalog,
    ) -> Result<CatalogActivation> {
        if request.expected_generation < 0 {
            return Err(Error::invalid("invalid directory generation"));
        }
        self.store.restore(user, project, request).await
    }
}
