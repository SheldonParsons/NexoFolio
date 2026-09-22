//! Local sessions and project access. Timestamps on the wire are UTC RFC3339.
use crate::{ExternalIdentity, ExternalProject, LoginCredentials, UserProfile};
use async_trait::async_trait;
use nexofolio_common::{ProjectId, Result, Secret, UserId};
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

#[derive(Debug, Clone)]
pub struct SessionPrincipal {
    pub user_id: UserId,
    pub instance: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectCard {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub can_access: bool,
    pub access_state: AccessState,
    pub reason_code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ProjectPage {
    pub items: Vec<ProjectCard>,
    pub page: u32,
    pub limit: u32,
    pub total: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct SyncCounts {
    pub created: u64,
    pub updated: u64,
    pub skipped: u64,
}

#[async_trait]
pub trait PlatformAccess: Send + Sync {
    async fn list_environments(
        &self,
        principal: &SessionPrincipal,
        project: nexofolio_common::ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<EnvironmentPage>;
    async fn create_environment(
        &self,
        principal: &SessionPrincipal,
        project: nexofolio_common::ProjectId,
        name: &str,
    ) -> Result<crate::Environment>;
    async fn rename_environment(
        &self,
        principal: &SessionPrincipal,
        project: nexofolio_common::ProjectId,
        id: nexofolio_common::EnvironmentId,
        name: &str,
    ) -> Result<crate::Environment>;

    async fn normal_login(&self, identity: &ExternalIdentity) -> Result<SessionLogin>;
    async fn emergency_login(&self, instance: &str, account: &str) -> Result<SessionLogin>;
    async fn next_sync_generation(&self) -> Result<i64>;
    async fn apply_snapshot(
        &self,
        principal: &SessionPrincipal,
        generation: i64,
        snapshot: ProjectSnapshot,
    ) -> Result<SyncCounts>;
    async fn verify_session(&self, token: &Secret) -> Result<SessionPrincipal>;
    async fn me(&self, principal: &SessionPrincipal) -> Result<UserProfile>;
    async fn list_projects(
        &self,
        principal: &SessionPrincipal,
        page: u32,
        limit: u32,
    ) -> Result<ProjectPage>;
    /// Every protected project resource must call this gate (not just its entry page).
    async fn require_project(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
    ) -> Result<ProjectCard>;
}

#[derive(Debug, Serialize)]
pub struct EnvironmentPage {
    pub items: Vec<crate::Environment>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}
