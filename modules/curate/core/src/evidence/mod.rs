//! Bounded observations, without model interpretation or rule evaluation.
mod context;
mod index;
mod samples;
pub use context::authentication;
pub use index::SignalIndex;
pub use samples::{SampleField, sample_fields};

mod headers;
pub(crate) use headers::header_evidence;
