use crate::*;
use async_trait::async_trait;
use nexofolio_contracts::Result;

pub trait CaptureValidator: Send + Sync {
    fn validate(&self, capture: &CaptureInput) -> Result<ValidationResult>;
}
pub trait IdentityResolver: Send + Sync {
    fn resolve(&self, capture: &CaptureInput) -> Result<InterfaceIdentity>;
}
pub trait Fingerprinter: Send + Sync {
    fn fingerprint(&self, capture: &CaptureInput) -> Result<StructuralFingerprint>;
}
#[async_trait]
pub trait Deduplicator: Send + Sync {
    async fn lookup(
        &self,
        identity: &InterfaceIdentity,
        fingerprint: &StructuralFingerprint,
    ) -> Result<DeduplicationResult>;
}
