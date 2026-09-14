//! Small, transport-independent types shared across component boundaries.
mod error;
mod events;
mod ids;
mod secret;

pub use error::{Error, Result};
pub use events::{KnowledgeEvent, KnowledgeEventKind, RebuildScope};
pub use ids::*;
pub use secret::Secret;
