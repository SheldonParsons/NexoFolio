//! Transport composition. Authentication owns only login and session access.
#[derive(Clone)]
pub struct BackendServices {
    pub access: super::access::AccessHttp,
    pub maintenance: Option<super::maintenance::MaintenanceHttp>,
    pub ingestion: Option<super::ingestion::IngestionHttp>,
    pub documents: Option<super::documents::DocumentsHttp>,
    pub official_catalog: Option<super::official_catalog::OfficialCatalogHttp>,
    pub catalog_previews: Option<super::catalog_preview::CatalogPreviewHttp>,
}
