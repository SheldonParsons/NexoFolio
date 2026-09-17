//! Cross-component orchestration and transactional ports; never concrete infrastructure.
mod organization;
mod ports;
pub use organization::*;
pub use ports::*;

mod login;
pub use login::*;

mod ingestion;
pub use ingestion::*;

mod processing;
pub use processing::*;

mod catalog_publication;
pub use catalog_publication::*;
mod knowledge_publication;
pub use knowledge_publication::*;
mod maintenance;
pub use maintenance::*;
mod maintenance_engine;
pub use maintenance_engine::*;

mod capture;
pub use capture::*;

/// Removed generation APIs cannot reappear unnoticed alongside MaintenanceEngine.
/// ```compile_fail
/// use nexofolio_application::CatalogPreviewService;
/// ```
const _: () = ();
