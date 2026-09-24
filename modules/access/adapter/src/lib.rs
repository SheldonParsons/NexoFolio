//! Access-owned persistence and ZenTao adapters. Database pool stays private.
mod postgres;
mod session_crypto;
mod store;
mod unconfigured;
mod zentao;
pub use postgres::Postgres;
pub use session_crypto::ConfiguredEmergencyPassword;
pub use store::PostgresAccess;
pub use unconfigured::Unconfigured;
pub use zentao::Zentao;

/// Concrete persistence details cannot be reached from other crates.
/// ```compile_fail
/// use nexofolio_access_adapter::store;
/// ```
/// ```compile_fail
/// fn bypass(store: nexofolio_access_adapter::Postgres) { let _ = store.pool; }
/// ```
const _: () = ();
