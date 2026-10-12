//! Read-only directory of users registered in the authenticated instance.
use crate::SessionPrincipal;
use async_trait::async_trait;
use nexofolio_common::{Error, Result, UserId};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ManagementUser {
    pub id: UserId,
    pub account: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub last_login_at: String,
}

#[derive(Debug, Serialize)]
pub struct ManagementUsers {
    pub items: Vec<ManagementUser>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

pub fn validate_user_directory_query(page: u32, limit: u32, query: &str) -> Result<()> {
    if page == 0
        || page > 100_000
        || !(1..=100).contains(&limit)
        || query.chars().count() > 100
        || query.chars().any(char::is_control)
    {
        return Err(Error::invalid("invalid user directory query"));
    }
    Ok(())
}

#[async_trait]
pub trait UserDirectory: Send + Sync {
    /// Requires the canonical management identity; never includes credentials or grants.
    async fn list_registered_users(
        &self,
        principal: &SessionPrincipal,
        page: u32,
        limit: u32,
        query: &str,
    ) -> Result<ManagementUsers>;
}
