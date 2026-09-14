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
