use async_trait::async_trait;
use nexofolio_contracts::Result;

#[async_trait]
pub trait DatabaseProbe: Send + Sync {
    async fn check(&self) -> Result<()>;
}
