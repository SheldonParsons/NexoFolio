//! Personal presentation belongs to NexoFolio; provider identity and grants do not.
use crate::{SessionPrincipal, UserProfile};
use async_trait::async_trait;
use nexofolio_common::Result;
use serde::{Deserialize, Serialize};
pub const PROFILE_NAME_LIMIT: usize = 40;
pub const PROFILE_BIO_LIMIT: usize = 160;
pub const AVATAR_MAX_BYTES: usize = 256 * 1024;
pub const AVATAR_SIZE: u32 = 512;

#[derive(Debug, Clone, Serialize)]
pub struct PersonalProfile {
    pub user: UserProfile,
    pub account_display_name: String,
    pub display_name_override: Option<String>,
    pub bio: String,
    pub version: i64,
    pub created_at: String,
    pub last_login_at: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdate {
    pub display_name: Option<String>,
    pub bio: String,
    pub version: i64,
}
#[async_trait]
pub trait PersonalProfiles: Send + Sync {
    async fn personal_profile(&self, principal: &SessionPrincipal) -> Result<PersonalProfile>;
    async fn update_profile(
        &self,
        principal: &SessionPrincipal,
        update: ProfileUpdate,
    ) -> Result<PersonalProfile>;
    /// Only decoded WebP content; transport base64 and image processing stay in the adapter.
    async fn update_avatar(
        &self,
        principal: &SessionPrincipal,
        version: i64,
        bytes: Option<Vec<u8>>,
    ) -> Result<PersonalProfile>;
}
