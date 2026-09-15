//! Concrete adapters. Unavailable business integrations fail explicitly.
mod postgres;
mod unconfigured;
pub use postgres::Postgres;
pub use unconfigured::Unconfigured;

mod platform_store;
mod session_crypto;
mod zentao;
pub use platform_store::PostgresAccess;
pub use session_crypto::ConfiguredEmergencyPassword;
pub use zentao::Zentao;

mod admission;
mod ingestion_heads;
pub use admission::PostgresAdmission;

mod environments;
mod project_access;

mod documents;
pub use documents::PostgresDocuments;

mod catalog_preview;
pub use catalog_preview::PostgresCatalogPreviews;

mod catalog_model;
mod chat_transport;
pub use catalog_model::ChatCatalogGenerator;

mod path_policy;
pub use path_policy::PostgresPathPolicies;

mod official_catalog;
pub use official_catalog::PostgresOfficialCatalog;

mod blob_store;
pub use blob_store::FileBlobStore;

mod capture_store;
pub use capture_store::PostgresCaptureStore;

mod evidence_processing;

mod evidence_retention;
mod maintenance_store;
pub use maintenance_store::PostgresMaintenance;
mod knowledge_publication;
mod maintenance_model;
pub use maintenance_model::ChatMaintenanceModel;
