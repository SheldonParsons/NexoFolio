//! Intake-owned persistence: the batch ledger. The database pool stays private.
mod postgres;
pub use postgres::PostgresLedger;

/// Concrete persistence details cannot be reached from other crates.
/// ```compile_fail
/// use nexofolio_intake_adapter::postgres;
/// ```
/// ```compile_fail
/// fn bypass(ledger: nexofolio_intake_adapter::PostgresLedger) { let _ = ledger.pool; }
/// ```
const _: () = ();
