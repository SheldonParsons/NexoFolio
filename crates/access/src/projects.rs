use async_trait::async_trait;
use nexofolio_contracts::{ProjectId, Result, UserId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExternalProject {
    pub instance: String,
    pub external_id: String,
    pub name: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct ProjectSourcePage {
    pub items: Vec<ExternalProject>,
    pub complete: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAction {
    Read,
    Write,
    Adjudicate,
    Publish,
    IssueMcpToken,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectGrant {
    pub project_id: ProjectId,
    pub actions: Vec<ProjectAction>,
}

/// Project creation is intentionally absent: projects come from the external source.
#[async_trait]
pub trait ProjectSource: Send + Sync {
    async fn list_projects(&self, cursor: Option<&str>) -> Result<ProjectSourcePage>;
}

/// Implementation must follow the confirmed ZenTao permission contract.
#[async_trait]
pub trait ProjectPermissionSource: Send + Sync {
    async fn grants_for(&self, user_id: UserId) -> Result<Vec<ProjectGrant>>;
}
