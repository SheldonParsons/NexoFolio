//! Contracts for the bounded capture hot path. No model, queue or reconstruction calls.
mod models;
mod ports;
pub use models::*;
pub use ports::*;

mod batch;
mod http_exchange;
pub use batch::*;
pub use http_exchange::{
    apply_path_policy, http_projection, http_projection_covers, prepare_capture, prepare_http,
};
