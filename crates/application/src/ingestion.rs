use nexofolio_access::{PlatformAccess, SessionPrincipal};
use nexofolio_contracts::Result;
use nexofolio_intake::{
    Admission, AdmissionResult, AdmissionStore, IngestionBatch, PreparedRecord,
};
use std::sync::Arc;

pub struct IngestionService {
    access: Arc<dyn PlatformAccess>,
    store: Arc<dyn AdmissionStore>,
}
impl IngestionService {
    pub fn new(access: Arc<dyn PlatformAccess>, store: Arc<dyn AdmissionStore>) -> Self {
        Self { access, store }
    }
    pub async fn authorize(
        &self,
        p: &SessionPrincipal,
        project: nexofolio_contracts::ProjectId,
    ) -> Result<()> {
        self.access.require_project(p, project).await.map(|_| ())
    }
    pub async fn admit(
        &self,
        p: &SessionPrincipal,
        batch: IngestionBatch,
        records: Vec<PreparedRecord>,
    ) -> Result<AdmissionResult> {
        // This is admission only. No triggers, builder, or fake downstream completion.
        self.store
            .admit(Admission {
                actor: p.user_id,
                instance: p.instance.clone(),
                project_id: batch.project_id,
                batch_id: batch.batch_id,
                environment: batch.environment,
                legacy_service_key: batch.service_key,
                source: batch.source,
                records,
            })
            .await
    }
}
