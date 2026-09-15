//! Mechanical evidence extraction only. No network, model, directory writes, or database dependency.
mod extraction;
pub use extraction::*;

use async_trait::async_trait;
use nexofolio_contracts::{ProjectId, Result};
#[derive(Debug, Clone)]
pub struct BlobRef {
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
}
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, project: ProjectId, bytes: Vec<u8>, media_type: &str) -> Result<BlobRef>;
    async fn get(&self, project: ProjectId, hash: &str) -> Result<Vec<u8>>;
    async fn delete(&self, project: ProjectId, hash: &str) -> Result<()>;
}

/// Authenticated project reads and writes; raw storage and permission implementations stay outside.
#[async_trait]
pub trait CaptureEvidenceRepository: Send + Sync {
    async fn put_asset(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        id: uuid::Uuid,
        media: &str,
        bytes: Vec<u8>,
    ) -> Result<nexofolio_contracts::AssetReceipt>;
    async fn asset(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<(String, Vec<u8>)>;
    async fn observation(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<nexofolio_contracts::CaptureObservation>;
    async fn fact(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> Result<nexofolio_contracts::EvidenceFact>;
    async fn facts(
        &self,
        user: nexofolio_contracts::UserId,
        project: ProjectId,
        page: u32,
        limit: u32,
        environment: Option<nexofolio_contracts::EnvironmentId>,
    ) -> Result<nexofolio_contracts::EvidencePage>;
}

mod relations;
pub use relations::*;
