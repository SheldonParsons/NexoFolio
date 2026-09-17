use async_trait::async_trait;
use nexofolio_contracts::*;
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone)]
pub struct MaintenanceLease {
    pub run: MaintenanceRun,
    pub snapshot: KnowledgeSnapshot,
    pub generation: i64,
    pub settings_hash: String,
}
#[async_trait]
pub trait MaintenanceStore: Send + Sync {
    async fn start(
        &self,
        user: UserId,
        project: ProjectId,
        request: &StartMaintenance,
    ) -> Result<MaintenanceRun>;
    async fn get(&self, user: UserId, project: ProjectId, run: Uuid) -> Result<MaintenanceRun>;
    async fn list(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<MaintenancePage>;
    async fn snapshot(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
    ) -> Result<KnowledgeSnapshot>;
    async fn checkpoints(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        page: u32,
        limit: u32,
    ) -> Result<MaintenanceCheckpointPage>;
    async fn claim(&self, settings_hash: &str) -> Result<Option<MaintenanceLease>>;
    async fn renew(&self, lease: &MaintenanceLease) -> Result<()>;
    async fn call_count(&self, lease: &MaintenanceLease) -> Result<u32>;
    async fn checkpoint(
        &self,
        lease: &MaintenanceLease,
        checkpoint: &MaintenanceCheckpoint,
        coverage: &ReviewCoverage,
    ) -> Result<()>;
    async fn saved_checkpoints(
        &self,
        lease: &MaintenanceLease,
    ) -> Result<Vec<MaintenanceCheckpoint>>;
    /// Persist exact provider JSON before sending. Does not include authentication headers.
    async fn begin_call(
        &self,
        lease: &MaintenanceLease,
        request: &Value,
        limit: u32,
    ) -> Result<Uuid>;
    async fn end_call(&self, lease: &MaintenanceLease, call: Uuid, response: &Value) -> Result<()>;
    async fn finish(
        &self,
        lease: &MaintenanceLease,
        candidate: Option<&MaintenanceCandidate>,
        code: Option<&str>,
    ) -> Result<()>;
}
#[async_trait]
pub trait KnowledgeActivationStore: Send + Sync {
    async fn publish_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        run: Uuid,
        request: &PublishKnowledge,
    ) -> Result<KnowledgeActivation>;
    async fn restore_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        request: &RestoreKnowledge,
    ) -> Result<KnowledgeActivation>;
    async fn knowledge_versions(
        &self,
        user: UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<KnowledgeVersionPage>;
    async fn interface_knowledge(
        &self,
        user: UserId,
        project: ProjectId,
        interface: InterfaceId,
        environment: EnvironmentId,
    ) -> Result<InterfaceKnowledge>;
}
/// Separate policy boundary; production wiring enables only manual authorized publication.
pub trait KnowledgePublicationPolicy: Send + Sync {
    fn authorize_mode(&self, explicitly_requested: bool) -> Result<()>;
}
pub struct ManualKnowledgePublication;
impl KnowledgePublicationPolicy for ManualKnowledgePublication {
    fn authorize_mode(&self, explicitly_requested: bool) -> Result<()> {
        if explicitly_requested {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

#[async_trait]
pub trait MaintenanceSources: Send + Sync {
    async fn observation(&self, source: &SnapshotSource) -> Result<Value>;
    async fn image(&self, project: ProjectId, asset: Uuid) -> Result<Value>;
}
