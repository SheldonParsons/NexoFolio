use crate::UserProfile;
use async_trait::async_trait;
use nexofolio_common::{Result, Secret, UserId};

#[derive(Debug, Clone)]
pub struct SessionPrincipal {
    pub user_id: UserId,
    pub instance: String,
}

#[async_trait]
pub trait Sessions: Send + Sync {
    async fn verify_session(&self, token: &Secret) -> Result<SessionPrincipal>;
    async fn me(&self, principal: &SessionPrincipal) -> Result<UserProfile>;
}
