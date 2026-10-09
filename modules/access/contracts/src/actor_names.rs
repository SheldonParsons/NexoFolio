//! Minimal display identity for actors already referenced by authorized records.
use async_trait::async_trait;
use nexofolio_common::{Result, UserId};
#[derive(Debug, Clone)]
pub struct ActorName {
    pub id: UserId,
    pub display_name: String,
}
#[async_trait]
pub trait ActorNames: Send + Sync {
    async fn actor_names(&self, ids: &[UserId]) -> Result<Vec<ActorName>>;
}
