//! Compose business transports with an authenticated project-access capability.
use crate::{http::access::AccessHttp, wiring::Config};
use nexofolio_access::PlatformAccess;
use nexofolio_contracts::Result;
use nexofolio_infrastructure::{Postgres, PostgresAdmission};
use std::sync::Arc;
pub(super) fn build_services(
    config: &Config,
    database: Postgres,
    store: Arc<dyn PlatformAccess>,
    access: AccessHttp,
) -> Result<axum::Router> {
    let service = Arc::new(nexofolio_application::IngestionService::new(
        store.clone(),
        Arc::new(PostgresAdmission::new(database.clone())),
    ));
    let mut ingestion = crate::http::ingestion::IngestionHttp::new(service, store.clone())?;
    if config.capture_enabled {
        let blobs = Arc::new(nexofolio_infrastructure::FileBlobStore::new(
            config.blob_root.clone(),
        ));
        let captures = Arc::new(nexofolio_infrastructure::PostgresCaptureStore::new(
            database.clone(),
            blobs,
        ));
        ingestion.capture = Some(crate::http::capture::CaptureHttp::new(
            captures,
            store.clone(),
        )?);
    }
    let documents = crate::http::documents::DocumentsHttp {
        reader: Arc::new(nexofolio_infrastructure::PostgresDocuments::new(
            database.clone(),
        )),
        access: store.clone(),
    };
    let previews = crate::http::catalog_preview::CatalogPreviewHttp {
        reader: Arc::new(nexofolio_infrastructure::PostgresCatalogPreviews::new(
            database.clone(),
        )),
        access: store.clone(),
    };
    let official = Arc::new(nexofolio_infrastructure::PostgresOfficialCatalog::new(
        database.clone(),
    ));
    let publication = Arc::new(nexofolio_application::CatalogPublicationService::new(
        previews.reader.clone(),
        official.clone(),
    ));
    let official_http = crate::http::official_catalog::OfficialCatalogHttp {
        access: store.clone(),
        reader: official,
        publication,
    };
    let maintenance_store = Arc::new(
        nexofolio_infrastructure::PostgresMaintenance::with_snapshot_limit(
            database,
            config.maintenance_snapshot_bytes,
        ),
    );
    let maintenance = crate::http::maintenance::MaintenanceHttp {
        access: store.clone(),
        store: maintenance_store.clone(),
        publication: Arc::new(nexofolio_application::KnowledgePublicationService::new(
            maintenance_store.clone(),
            Arc::new(nexofolio_application::ManualKnowledgePublication),
        )),
        knowledge: maintenance_store,
        model_configured: config.catalog_model.is_some(),
    };
    let capture = ingestion
        .capture
        .clone()
        .map(crate::http::capture::routes)
        .unwrap_or_default();
    Ok(crate::http::access::routes(access)
        .merge(crate::http::ingestion::routes(ingestion))
        .merge(crate::http::documents::routes(documents))
        .merge(crate::http::catalog_preview::routes(previews))
        .merge(crate::http::official_catalog::routes(official_http))
        .merge(crate::http::maintenance::routes(maintenance))
        .merge(capture))
}
