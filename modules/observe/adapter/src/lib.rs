//! Observe-owned persistence. The database pool stays private.
mod postgres;
pub use postgres::PostgresObserve;

/// Concrete persistence details cannot be reached from other crates.
/// ```compile_fail
/// use nexofolio_observe_adapter::postgres;
/// ```
/// ```compile_fail
/// fn bypass(store: nexofolio_observe_adapter::PostgresObserve) { let _ = store.pool; }
/// ```
const _: () = ();
