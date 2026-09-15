//! Small, transport-independent types shared across component boundaries.
mod error;
mod events;
mod ids;
mod secret;

pub use error::{Error, Result};
pub use events::{KnowledgeEvent, KnowledgeEventKind, RebuildScope};
pub use ids::*;
pub use secret::Secret;

mod environment;
pub use environment::*;

mod observed_comparison;
pub use observed_comparison::observed_schema_covers;

mod catalog_preview;
pub use catalog_preview::*;

mod path_identity;
pub use path_identity::*;

mod official_catalog;
pub use official_catalog::*;

mod capture;
pub use capture::*;

mod maintenance;
pub use maintenance::*;
