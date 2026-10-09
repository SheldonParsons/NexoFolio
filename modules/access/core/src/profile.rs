use async_trait::async_trait;
use nexofolio_access_contracts::{
    AVATAR_MAX_BYTES, PROFILE_BIO_LIMIT, PROFILE_NAME_LIMIT, PersonalProfile, PersonalProfiles,
    ProfileUpdate, SessionPrincipal,
};
use nexofolio_common::{Error, Result};
use std::sync::Arc;
pub struct ProfileService {
    store: Arc<dyn PersonalProfiles>,
}
impl ProfileService {
    pub fn new(store: Arc<dyn PersonalProfiles>) -> Self {
        Self { store }
    }
}
pub fn normalize_profile(mut update: ProfileUpdate) -> Result<ProfileUpdate> {
    if update.version < 0 {
        return Err(Error::invalid("资料版本无效"));
    }
    update.display_name = update
        .display_name
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if update
        .display_name
        .as_ref()
        .is_some_and(|s| s.chars().count() > PROFILE_NAME_LIMIT || s.chars().any(char::is_control))
    {
        return Err(Error::invalid("显示名称最多 40 字，不能包含控制字符"));
    }
    update.bio = update.bio.trim().to_owned();
    if update.bio.chars().count() > PROFILE_BIO_LIMIT
        || update.bio.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(Error::invalid("个人简介最多 160 字"));
    }
    Ok(update)
}
#[async_trait]
impl PersonalProfiles for ProfileService {
    async fn personal_profile(&self, p: &SessionPrincipal) -> Result<PersonalProfile> {
        self.store.personal_profile(p).await
    }
    async fn update_profile(
        &self,
        p: &SessionPrincipal,
        update: ProfileUpdate,
    ) -> Result<PersonalProfile> {
        self.store
            .update_profile(p, normalize_profile(update)?)
            .await
    }
    async fn update_avatar(
        &self,
        p: &SessionPrincipal,
        version: i64,
        bytes: Option<Vec<u8>>,
    ) -> Result<PersonalProfile> {
        if version < 0 {
            return Err(Error::invalid("资料版本无效"));
        }
        if bytes.as_ref().is_some_and(|b| b.len() > AVATAR_MAX_BYTES) {
            return Err(Error::invalid("头像需小于 256 KiB"));
        }
        self.store.update_avatar(p, version, bytes).await
    }
}
