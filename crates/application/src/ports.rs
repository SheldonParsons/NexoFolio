use crate::JobRequest;
use async_trait::async_trait;
use nexofolio_contracts::{
    ChangeId, InterfaceId, KnowledgeEvent, ProjectId, Result, RevisionId, UserId,
};
use nexofolio_knowledge::InterfaceRevision;

#[derive(Debug)]
pub struct CommitRevision {
    pub project_id: ProjectId,
    pub interface_id: InterfaceId,
    pub actor: UserId,
    pub base_revision: RevisionId,
    pub revision: InterfaceRevision,
    pub event: KnowledgeEvent,
    pub outbox: Vec<JobRequest>,
    pub idempotency_key: String,
}

#[async_trait]
pub trait KnowledgeTransaction: Send + Sync {
    /// Validate revision, commit data/event/outbox and receipt atomically.
    async fn commit_revision(&self, command: CommitRevision) -> Result<ChangeId>;
}

#[async_trait]
pub trait DatabaseProbe: Send + Sync {
    async fn check(&self) -> Result<()>;
}
