use crate::SessionPrincipal;
use async_trait::async_trait;
use nexofolio_common::{EnvironmentId, Error, ProjectId, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct EnvironmentPage {
    pub items: Vec<Environment>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

/// Exact case-sensitive names; no silent Unicode/case/whitespace normalization.
pub fn validate_environment_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 64
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput {
            message:
                "environment name must be 1..64 characters, no controls or surrounding whitespace"
                    .into(),
        });
    }
    Ok(())
}

/// Environments of a project the principal may access. Renaming keeps the old
/// name as an alias, so uploads that still carry it resolve to the same environment.
#[async_trait]
pub trait Environments: Send + Sync {
    async fn list_environments(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        page: u32,
        limit: u32,
    ) -> Result<EnvironmentPage>;
    async fn create_environment(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        name: &str,
    ) -> Result<Environment>;
    async fn rename_environment(
        &self,
        principal: &SessionPrincipal,
        project: ProjectId,
        id: EnvironmentId,
        name: &str,
    ) -> Result<Environment>;
}
