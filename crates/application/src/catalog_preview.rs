use nexofolio_contracts::{Error, JobId, PreviewTask, ProjectId, Result};
use nexofolio_knowledge::CatalogPreviewStore;
use nexofolio_rebuild::{DirectoryGenerator, DirectoryReviewer};
use std::sync::Arc;

pub struct CatalogPreviewService {
    store: Arc<dyn CatalogPreviewStore>,
    generator: Arc<dyn DirectoryGenerator>,
    reviewer: Arc<dyn DirectoryReviewer>,
}
impl CatalogPreviewService {
    pub fn new(
        store: Arc<dyn CatalogPreviewStore>,
        generator: Arc<dyn DirectoryGenerator>,
        reviewer: Arc<dyn DirectoryReviewer>,
    ) -> Self {
        Self {
            store,
            generator,
            reviewer,
        }
    }
    pub async fn create(&self, project: ProjectId) -> Result<PreviewTask> {
        self.store.create(project).await
    }
    pub async fn read(&self, task: JobId) -> Result<PreviewTask> {
        self.store.read(task).await
    }
    pub async fn run(&self, id: JobId) -> Result<PreviewTask> {
        let info = self.generator.info()?;
        let task = self.store.claim(id, &info).await?;
        let candidate = match self.generator.generate(&task.snapshot).await {
            Ok(candidate) => candidate,
            Err(error) => {
                let code = match error {
                    Error::NotConfigured { .. } => "MODEL_NOT_CONFIGURED",
                    Error::InvalidInput { .. } => "MODEL_INVALID_RESULT",
                    _ => "MODEL_UNAVAILABLE",
                };
                self.store.fail(&task, code).await?;
                return Err(error);
            }
        };
        let review = self.reviewer.review(&task.snapshot, &candidate);
        self.store.complete(&task, &candidate, &review).await?;
        self.store.read(id).await
    }
}

impl CatalogPreviewService {
    pub fn standard(
        store: Arc<dyn CatalogPreviewStore>,
        generator: Arc<dyn DirectoryGenerator>,
    ) -> Self {
        Self::new(
            store,
            generator,
            Arc::new(nexofolio_rebuild::StructuralDirectoryReviewer),
        )
    }
}
