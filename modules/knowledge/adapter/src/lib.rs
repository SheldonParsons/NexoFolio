//! Knowledge-owned persistence. The database pool stays private.
mod postgres;
pub use postgres::PostgresKnowledge;

/// Concrete persistence details cannot be reached from other crates.
/// ```compile_fail
/// use nexofolio_knowledge_adapter::postgres;
/// ```
/// ```compile_fail
/// fn bypass(store: nexofolio_knowledge_adapter::PostgresKnowledge) { let _ = store.pool; }
/// ```
const _: () = ();
