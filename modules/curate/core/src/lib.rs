//! Curate derives updateable knowledge from collected facts.
//!
//! Evidence extracts bounded observations; rules find deterministic clues;
//! planning decides freshness and pass dependencies; interpretation calls the
//! model and proposes Commands. None of these modules reads SQL or schedules IO.
mod evidence;
mod interpretation;
mod planning;
mod rules;

pub use evidence::{SampleField, SignalIndex, authentication, sample_fields};
pub use interpretation::{Describe, Organize};
pub use nexofolio_curate_contracts::{CompletionError, Completions};
pub use planning::ProjectEvidence;
pub use planning::field_addressed;
mod value_budget;
pub use planning::{
    FieldInput, Pass, field_input_hash, field_inputs, legacy_field_input_hash, pending_fields,
    workflow,
};
pub use value_budget::model_value_examples;

pub use rules::{LinkProposal, NAVIGATION_RECIPE, navigation_links};

pub use planning::{
    EndpointInput, description_context, description_pending, endpoint_input,
    endpoint_input_from_fields, endpoint_input_hash,
};

pub use interpretation::FieldBatchOutput;
pub use planning::directory_input_hash;
pub use planning::relation_commands;
pub use planning::{FIELD_BATCH_BYTES, FIELD_BATCH_LIMIT, field_batches};

pub use planning::{TraceOptions, TraceResult, TraceStep, trace_environment, trace_upstream};

pub use interpretation::standard_field_command;
