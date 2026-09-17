mod config;
mod lifecycle;
pub use config::Config;
pub use lifecycle::{
    install_shutdown_handler, run_evidence_worker, run_processing_worker, run_worker,
};

use nexofolio_contracts::{Error, Result};
use tracing_subscriber::EnvFilter;

pub fn init_logging(filter: &str) -> Result<()> {
    let filter =
        EnvFilter::try_new(filter).map_err(|_| Error::invalid("NEXOFOLIO_LOG is invalid"))?;
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|_| Error::Unavailable {
            component: "logging",
        })
}

mod access;
mod services;
pub use access::build_access;

mod maintenance;
pub use lifecycle::run_maintenance_worker;
pub use maintenance::build_maintenance;
