//! What curate needs from the outside: one turn with a language model.
//!
//! Curate's passes are pure functions of observe's facts plus a model reply, so
//! the only IO they need is this. The HTTP implementation lives in the adapter;
//! tests substitute a canned reply and never touch a network.

use async_trait::async_trait;
pub mod telemetry;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompletionError {
    #[error("模型请求配置无效，请修复配置后继续")]
    Configuration,
    #[error("{0}")]
    InvalidReply(String),
    #[error("模型连接暂时不可用")]
    Network,
    #[error("模型请求超时")]
    Timeout,
    #[error("模型服务返回 HTTP {status}")]
    Status {
        status: u16,
        retry_after_seconds: Option<u64>,
    },
    #[error("模型服务正在冷却，等待后自动恢复")]
    Cooldown { retry_after_seconds: u64 },
}
impl CompletionError {
    pub fn temporary(&self) -> bool {
        matches!(
            self,
            Self::Network
                | Self::Timeout
                | Self::Cooldown { .. }
                | Self::Status {
                    status: 408 | 429 | 500..=599,
                    ..
                }
        )
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Configuration => "model_configuration",
            Self::Network => "model_network",
            Self::Timeout | Self::Status { status: 408, .. } => "model_timeout",
            Self::Status { status: 429, .. } => "model_rate_limited",
            Self::Status {
                status: 500..=599, ..
            } => "model_unavailable",
            Self::Status { .. } => "model_rejected",
            Self::Cooldown { .. } => "model_cooldown",
            Self::InvalidReply(_) => "invalid_reply",
        }
    }
    pub fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Self::Status {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            Self::Cooldown {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            _ => None,
        }
    }
}

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
