//! Canonical knowledge and catalog contracts; persistence and algorithms are external.
mod ports;
pub use ports::*;

mod observed;
pub use observed::*;

mod catalog_preview;
pub use catalog_preview::*;
