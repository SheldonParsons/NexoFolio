use nexofolio_common::{Error, ProjectId, Result};
use serde::Serialize;

/// Canonical, enabled identity only. Never use an account supplied in an operation request.
pub fn can_manage_projects(account: &str, enabled: bool) -> bool {
    enabled && account.eq_ignore_ascii_case("a80646")
}

pub fn validate_project_name(name: &str) -> Result<()> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > 100
        || name.chars().any(char::is_control)
    {
        return Err(Error::invalid(
            "project name must contain 1..100 characters",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionSource {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct PermissionSources {
    pub items: Vec<PermissionSource>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Serialize)]
pub struct ManagedProject {
    pub project_id: ProjectId,
    pub name: String,
    /// Compatibility projection for clients creating a single-source project.
    pub permission_source: PermissionSource,
    pub permission_sources: Vec<PermissionSource>,
    pub revision: i64,
}

pub fn validate_permission_sources(sources: &[ProjectId]) -> Result<()> {
    let unique: std::collections::HashSet<_> = sources.iter().collect();
    if sources.is_empty() || sources.len() > 100 || unique.len() != sources.len() {
        return Err(Error::invalid("select 1..100 distinct permission sources"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_uses_enabled_exact_case_insensitive_identity() {
        assert!(can_manage_projects("A80646", true));
        assert!(can_manage_projects("a80646", true));
        for name in ["XA80646", "A806460", " A80646", "A80646 ", "member"] {
            assert!(!can_manage_projects(name, true));
        }
        assert!(!can_manage_projects("a80646", false));
    }
    #[test]
    fn project_names_are_bounded_by_characters_and_never_contain_control_characters() {
        assert!(validate_project_name(&"项".repeat(100)).is_ok());
        for name in [
            "".to_owned(),
            "  ".to_owned(),
            "bad\nname".to_owned(),
            "项".repeat(101),
        ] {
            assert!(validate_project_name(&name).is_err());
        }
    }
}

/// Management-only projection; permission bindings are batch-loaded for the current page.
#[derive(Debug, Serialize)]
pub struct ManagementProject {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub origin: String,
    pub permission_sources: Vec<PermissionSource>,
}
#[derive(Debug, Serialize)]
pub struct ManagementProjects {
    pub items: Vec<ManagementProject>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}
