//! Minimal technical primitives. Business contracts belong to their owning module.
mod error;
mod ids;
mod ports;
mod secret;
mod telemetry;
pub use error::{Error, Result};
pub use ids::*;
pub use ports::DatabaseProbe;
pub use secret::Secret;
pub use telemetry::{PoolSnapshot, PoolTelemetry};

pub mod storage;
