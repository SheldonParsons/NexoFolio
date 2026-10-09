//! Which knowledge needs updating, and the prerequisites between passes.
pub(crate) mod catalogue;
mod endpoints;
mod field_batches;
mod shared_fields;
pub(crate) use shared_fields::{shared_endpoint_input, shared_fields};
mod fields;
pub(crate) use field_batches::model_field;
pub use field_batches::{FIELD_BATCH_BYTES, FIELD_BATCH_LIMIT, field_batches};
mod inputs;
mod project_evidence;
pub use project_evidence::ProjectEvidence;
mod directory_freshness;
mod workflow;
pub use directory_freshness::directory_input_hash;

pub use fields::{field_input_hash, legacy_field_input_hash, pending_fields};
pub use inputs::{FieldInput, field_inputs};
pub use workflow::{Pass, workflow};

pub(crate) use endpoints::endpoint_needs_update;

mod endpoint_input;
pub use endpoint_input::*;

mod links;
pub use links::relation_commands;

mod trace;
pub use trace::*;
mod trace_environment;
pub use trace_environment::trace_environment;

pub use fields::field_addressed;
