use crate::SessionPrincipal;
use async_trait::async_trait;
use nexofolio_common::{ProjectId, Result};
use serde::Serialize;

/// A project as ZenTao reports it. Projects are never created locally.
#[derive(Debug, Clone)]
pub struct ExternalProject {
    pub instance: String,
    pub external_id: String,
    pub name: String,
    pub state: String,
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

#[async_trait]
pub trait ProjectAccess: Send + Sync {
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
