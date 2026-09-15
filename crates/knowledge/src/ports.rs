use crate::{InterfaceRevision, PublicationCondition};
use async_trait::async_trait;
use nexofolio_contracts::{InterfaceId, ProjectId, Result, RevisionId};

#[async_trait]
pub trait KnowledgeReader: Send + Sync {
    async fn read(
        &self,
        project: ProjectId,
        interface: InterfaceId,
        revision: Option<RevisionId>,
    ) -> Result<InterfaceRevision>;
}

#[async_trait]
pub trait CatalogPublisher: Send + Sync {
    /// Must atomically verify ALL conditions; generating a candidate never publishes it.
    async fn publish(&self, condition: PublicationCondition) -> Result<()>;
}

#[async_trait]
pub trait OfficialCatalogReader: Send + Sync {
    async fn current(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
    ) -> Result<nexofolio_contracts::OfficialCatalog>;
    async fn interfaces(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        directory: Option<nexofolio_contracts::DirectoryId>,
        page: u32,
        limit: u32,
        expected_generation: Option<i64>,
    ) -> Result<nexofolio_contracts::OfficialInterfacePage>;
    async fn versions(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<nexofolio_contracts::CatalogVersionPage>;
}
