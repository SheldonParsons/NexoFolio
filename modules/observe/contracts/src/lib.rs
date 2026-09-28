//! What observe core and its storage adapter share: the rows observe keeps and
//! the transactional store that keeps them.
//!
//! Core decides everything (addresses, templates, structures, labels); the
//! store only remembers. Structures and samples are opaque JSON to the store.
//! One [`ObserveTx`] covers one delivered batch, so a batch counts fully or
//! not at all.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::endpoint::{EndpointEvent, EnvironmentUsage, ServiceAddress, Verdict};
use serde_json::Value;
use uuid::Uuid;

#[cfg(feature = "testing")]
pub mod testing;

/// What a project knows about one service address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressRow {
    pub address: ServiceAddress,
    pub manual: Option<Verdict>,
    pub calls: u64,
    /// `None` before any traffic.
    pub last_seen: Option<DateTime<Utc>>,
}

impl AddressRow {
    /// Every address is the project's own until someone decides otherwise.
    pub fn verdict(&self) -> Verdict {
        self.manual.unwrap_or(Verdict::Own)
    }
}

/// The leading path an address serves every endpoint under, per environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasePath {
    pub environment_id: EnvironmentId,
    pub address: ServiceAddress,
    pub base_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointRow {
    pub id: EndpointId,
    pub project_id: ProjectId,
    pub method: String,
    pub path_template: String,
}

/// Calls of one endpoint in one environment on one address: the unit that
/// moves when an endpoint's identity changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Traffic {
    pub endpoint: EndpointId,
    pub environment_id: EnvironmentId,
    pub address: ServiceAddress,
}

pub type FingerprintHash = [u8; 32];

#[derive(Debug, Clone, PartialEq)]
pub struct NewFingerprint {
    pub traffic: Traffic,
    pub hash: FingerprintHash,
    pub structure: Value,
    /// The first observation with this structure, kept whole.
    pub sample: Value,
    pub seen: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FingerprintStats {
    pub environment_id: EnvironmentId,
    pub address: ServiceAddress,
    pub structure: Value,
    pub calls: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

/// One source's declaration of an endpoint. A newer one from the same
/// platform + source URL + environment replaces it.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// `None`: applies to every environment.
    pub environment_id: Option<EnvironmentId>,
    pub platform: String,
    pub source_url: Option<String>,
    pub structure: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Retryable: storage is down.
    #[error("observe storage unavailable")]
    Unavailable,
}

pub type StoreResult<T> = Result<T, StoreError>;

#[async_trait]
pub trait ObserveStore: Send + Sync {
    /// Dropping the transaction without [`ObserveTx::commit`] undoes it.
    async fn begin(&self) -> StoreResult<Box<dyn ObserveTx>>;

    /// Forgets delivered batches seen before `before`; returns how many.
    async fn purge_batches(&self, before: DateTime<Utc>) -> StoreResult<u64>;
}

#[async_trait]
pub trait ObserveTx: Send {
    async fn commit(self: Box<Self>) -> StoreResult<()>;

    /// Serialises writers of one project until the transaction ends.
    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()>;

    /// Records a delivered batch; `false` when it was delivered before.
    async fn first_delivery(&mut self, batch_id: Uuid) -> StoreResult<bool>;

    async fn address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
    ) -> StoreResult<Option<AddressRow>>;

    /// Ordered by address.
    async fn addresses(&mut self, project: ProjectId) -> StoreResult<Vec<AddressRow>>;

    async fn count_address(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        at: DateTime<Utc>,
    ) -> StoreResult<()>;

    /// `None` clears the manual verdict; an address left without a verdict
    /// or traffic is forgotten.
    async fn set_manual(
        &mut self,
        project: ProjectId,
        address: &ServiceAddress,
        verdict: Option<Verdict>,
    ) -> StoreResult<()>;

    async fn base_paths(&mut self, project: ProjectId) -> StoreResult<Vec<BasePath>>;

    async fn set_base_path(&mut self, project: ProjectId, base: &BasePath) -> StoreResult<()>;

    /// A current endpoint by identity; aliases never match.
    async fn find_endpoint(
        &mut self,
        project: ProjectId,
        method: &str,
        path_template: &str,
    ) -> StoreResult<Option<EndpointId>>;

    async fn create_endpoint(&mut self, endpoint: &EndpointRow) -> StoreResult<()>;

    /// Current endpoints of a project.
    async fn endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>>;

    /// Current endpoints of a project that have at least one declaration.
    async fn declared_endpoints(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointRow>>;

    /// The endpoint, or the one an alias was merged into.
    async fn endpoint(&mut self, id: EndpointId) -> StoreResult<Option<EndpointRow>>;

    async fn aliases(&mut self, id: EndpointId) -> StoreResult<Vec<EndpointId>>;

    /// Makes `from`, and every alias of it, an alias of `into`.
    async fn alias(&mut self, from: EndpointId, into: EndpointId) -> StoreResult<()>;

    /// Counts one more call of a known fingerprint; `false` when it is new.
    async fn count_fingerprint(
        &mut self,
        traffic: &Traffic,
        hash: &FingerprintHash,
        at: DateTime<Utc>,
    ) -> StoreResult<bool>;

    async fn insert_fingerprint(&mut self, fingerprint: &NewFingerprint) -> StoreResult<()>;

    async fn fingerprints(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FingerprintStats>>;

    /// Every traffic group of the project's current endpoints.
    async fn traffic(&mut self, project: ProjectId) -> StoreResult<Vec<Traffic>>;

    /// Per current endpoint and environment, summed over its fingerprints.
    async fn usage(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Vec<(EndpointId, EnvironmentUsage)>>;

    /// Moves every fingerprint of `from` to `to`, adding up equal ones.
    async fn move_traffic(&mut self, from: &Traffic, to: EndpointId) -> StoreResult<()>;

    /// Stores or replaces; `false` when nothing changed, including when the
    /// stored declaration is newer.
    async fn put_declaration(
        &mut self,
        endpoint: EndpointId,
        declaration: &Declaration,
    ) -> StoreResult<bool>;

    async fn declarations(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Declaration>>;

    /// Appends to the outbox. Call once, right before commit: readers must
    /// never see a later cursor commit before an earlier one.
    async fn publish(&mut self, events: &[EndpointEvent]) -> StoreResult<()>;
}
