//! The only shared contract layer between backend modules.
//!
//! Modules depend on this crate and on `nexofolio-common`, never on each other.
//! Everything here is types, traits and errors: no IO, no storage, no runtime.
//! `apps/backend` picks the implementations and wires them together.
//!
//! | contract      | implemented by | used by      |
//! |---------------|----------------|--------------|
//! | [`scope`]       | access         | intake, apps |
//! | [`observation`] | observe        | intake       |
//!
//! Enable the `testing` feature for in-memory fakes and the conformance suites
//! that every real implementation must pass.

pub mod observation;
pub mod scope;

#[cfg(feature = "testing")]
pub mod testing;
