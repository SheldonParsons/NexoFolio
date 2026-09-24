//! In-memory fakes and conformance suites (`testing` feature).
//!
//! A module under test wires fakes in place of the modules it talks to. A real
//! implementation proves it honours a contract by passing the same conformance
//! suite the fake passes.

mod observation;
mod scope;

pub use observation::{
    RecordingSink, observation_sink_conformance, sample_declaration, sample_exchange,
};
pub use scope::{
    InMemoryScope, ScopeFixture, site_registry_conformance, target_resolver_conformance,
};
