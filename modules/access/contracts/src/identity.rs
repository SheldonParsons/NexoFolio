use nexofolio_common::{Secret, UserId};
use serde::Serialize;

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

#[derive(Debug, Clone, Serialize)]
pub struct UserProfile {
    pub id: UserId,
    pub account: String,
    pub display_name: String,
    pub enabled: bool,
}
