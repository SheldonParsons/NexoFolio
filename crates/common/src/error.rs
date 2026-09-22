#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("capability not configured: {capability}")]
    NotConfigured { capability: &'static str },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("authentication required")]
    Unauthenticated,
    #[error("permission denied")]
    Forbidden,
    #[error("resource not found")]
    NotFound,
    #[error("component unavailable: {component}")]
    Unavailable { component: &'static str },
    #[error("version condition failed")]
    Conflict,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Construct an input error without repeating ownership conversion at every validation site.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }
}
