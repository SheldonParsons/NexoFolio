//! What curate needs from the outside: one turn with a language model.
//!
//! Curate's passes are pure functions of observe's facts plus a model reply, so
//! the only IO they need is this. The HTTP implementation lives in the adapter;
//! tests substitute a canned reply and never touch a network.

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CompletionError(pub String);

#[async_trait]
pub trait Completions: Send + Sync {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError>;
}

/// So a pass can be built over `Arc<dyn Completions>` and still be one concrete
/// type the HTTP layer can hold.
#[async_trait]
impl Completions for std::sync::Arc<dyn Completions> {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        (**self).complete(prompt).await
    }
}
