//! Signing in: ZenTao verifies the user, the store issues a local session and
//! records which projects ZenTao let the user see.
use crate::{ExternalIdentity, ExternalProject, LoginCredentials, SessionPrincipal, UserProfile};
use async_trait::async_trait;
use nexofolio_common::{Result, Secret};
use serde::Serialize;

pub struct RemoteLogin {
    pub identity: ExternalIdentity,
    pub credential: Secret,
    /// None means the instance omitted the field; Some(empty) is explicitly empty.
    pub visible_project_ids: Option<Vec<String>>,
}

pub struct ProjectSnapshot {
    pub projects: Vec<ExternalProject>,
    pub visible_project_ids: Vec<String>,
}

#[async_trait]
pub trait LoginProvider: Send + Sync {
    async fn login(&self, credentials: &LoginCredentials) -> Result<RemoteLogin>;
    /// Complete, bounded pagination only. A partial response is an error, never a snapshot.
    async fn projects(&self, login: &RemoteLogin) -> Result<ProjectSnapshot>;
}

#[async_trait]
pub trait EmergencyPassword: Send + Sync {
    async fn matches(&self, password: &Secret) -> Result<bool>;
}

pub struct SessionLogin {
    pub user: UserProfile,
    pub token: Secret,
    pub expires_at: String,
    pub reused: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct SyncCounts {
    pub created: u64,
    pub updated: u64,
    pub skipped: u64,
}

#[async_trait]
pub trait LoginStore: Send + Sync {
    async fn normal_login(&self, identity: &ExternalIdentity) -> Result<SessionLogin>;
    /// Resolves an existing local user only; never creates users or grants.
    async fn emergency_login(&self, instance: &str, account: &str) -> Result<SessionLogin>;
    /// Taken before contacting ZenTao, so an older snapshot never overwrites a newer one.
    async fn next_sync_generation(&self) -> Result<i64>;
    async fn apply_snapshot(
        &self,
        principal: &SessionPrincipal,
        generation: i64,
        snapshot: ProjectSnapshot,
    ) -> Result<SyncCounts>;
}
