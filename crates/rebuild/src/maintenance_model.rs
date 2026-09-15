use async_trait::async_trait;
use nexofolio_contracts::*;
use serde_json::Value;
#[derive(Debug)]
pub struct ModelInvocation {
    pub content: Value,
    /// Provider-reported usage; absent means unavailable, never zero.
    pub usage: Option<Value>,
}
/// Provider request construction is separate so the application can persist exactly what is sent.
#[async_trait]
pub trait MaintenanceModel: Send + Sync {
    fn identity(&self) -> Value;
    fn prepare(&self, phase: &str, input: Value, output_tokens: usize) -> Result<Value>;
    async fn invoke(&self, request: &Value) -> Result<Value>;
    async fn invoke_tracked(&self, request: &Value) -> Result<ModelInvocation> {
        Ok(ModelInvocation {
            content: self.invoke(request).await?,
            usage: None,
        })
    }
}
