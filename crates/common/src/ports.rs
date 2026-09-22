use crate::Result;
use async_trait::async_trait;

#[async_trait]
pub trait DatabaseProbe: Send + Sync {
    async fn check(&self) -> Result<()>;
}
