use crate::UserProfile;
use async_trait::async_trait;
use nexofolio_contracts::{Result, Secret};

#[derive(Debug)]
pub struct LoginCredentials {
    pub account: String,
    pub password: Secret,
}

#[derive(Debug, Clone)]
pub struct ExternalIdentity {
    pub instance: String,
    pub external_id: String,
    pub account: String,
    pub display_name: String,
}

#[async_trait]
pub trait ExternalAuthenticator: Send + Sync {
    async fn authenticate(&self, credentials: &LoginCredentials) -> Result<ExternalIdentity>;
}

/// Must resolve an existing local user. Never creates users or grants new permissions.
#[async_trait]
pub trait EmergencyAuthenticator: Send + Sync {
    async fn authenticate_existing(&self, credentials: &LoginCredentials) -> Result<UserProfile>;
}
