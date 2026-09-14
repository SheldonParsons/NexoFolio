use async_trait::async_trait;
use nexofolio_contracts::{Result, UserId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserProfile {
    pub id: UserId,
    pub account: String,
    pub display_name: String,
    pub enabled: bool,
}

#[async_trait]
pub trait UserDirectory: Send + Sync {
    async fn find_existing(&self, account: &str) -> Result<Option<UserProfile>>;
}
