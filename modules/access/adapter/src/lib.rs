//! Access-owned persistence and ZenTao adapters. Database pool stays private.
mod environments;
mod platform_store;
mod postgres;
mod session_crypto;
mod unconfigured;
mod zentao;
pub use platform_store::PostgresAccess;
pub use postgres::Postgres;
pub use session_crypto::ConfiguredEmergencyPassword;
pub use unconfigured::Unconfigured;
pub use zentao::Zentao;

/// Concrete persistence details cannot be reached from other crates.
/// ```compile_fail
/// use nexofolio_access_adapter::platform_store;
/// ```
/// ```compile_fail
/// fn bypass(store: nexofolio_access_adapter::Postgres) { let _ = store.pool; }
/// ```
const _: () = ();
