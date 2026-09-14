//! Concrete adapters. Unavailable business integrations fail explicitly.
mod postgres;
mod unconfigured;
pub use postgres::Postgres;
pub use unconfigured::Unconfigured;

mod platform_store;
mod session_crypto;
mod zentao;
pub use platform_store::PostgresAccess;
pub use session_crypto::ConfiguredEmergencyPassword;
pub use zentao::Zentao;
