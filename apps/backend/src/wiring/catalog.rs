use crate::wiring::Config;
use nexofolio_application::CatalogPreviewService;
use nexofolio_contracts::{Error, Result, Secret};
use nexofolio_infrastructure::{
    ChatCatalogGenerator, Postgres, PostgresCatalogPreviews, Unconfigured,
};
use std::sync::Arc;

pub fn build_catalog_previews(
    config: &Config,
    database: Postgres,
) -> Result<CatalogPreviewService> {
    let store = Arc::new(PostgresCatalogPreviews::new(database));
    match &config.catalog_model {
        None => Ok(CatalogPreviewService::standard(
            store,
            Arc::new(Unconfigured),
        )),
        Some(model) => {
            let key =
                std::fs::read_to_string(&model.api_key_file).map_err(|_| Error::NotConfigured {
                    capability: "catalog_model_key_file",
                })?;
            let key = Secret::new(key.trim());
            Ok(CatalogPreviewService::standard(
                store,
                Arc::new(ChatCatalogGenerator::new(
                    &model.base_url,
                    key,
                    model.model.clone(),
                )?),
            ))
        }
    }
}

pub fn build_maintenance(
    config: &Config,
    database: Postgres,
) -> Result<Option<nexofolio_application::MaintenanceEngine>> {
    let Some(model) = &config.catalog_model else {
        return Ok(None);
    };
    let key = std::fs::read_to_string(&model.api_key_file).map_err(|_| Error::NotConfigured {
        capability: "maintenance_model_key_file",
    })?;
    let model = Arc::new(
        nexofolio_infrastructure::ChatMaintenanceModel::with_timeout(
            &model.base_url,
            Secret::new(key.trim()),
            model.model.clone(),
            config.maintenance_vision,
            config.maintenance_model_timeout,
        )?,
    );
    let sources = Arc::new(nexofolio_infrastructure::PostgresCaptureStore::new(
        database.clone(),
        Arc::new(nexofolio_infrastructure::FileBlobStore::new(
            config.blob_root.clone(),
        )),
    ));
    Ok(Some(nexofolio_application::MaintenanceEngine {
        store: Arc::new(nexofolio_infrastructure::PostgresMaintenance::new(database)),
        model,
        sources,
        budget: nexofolio_application::MaintenanceBudget {
            context_tokens: config.maintenance_context_tokens,
            max_calls: config.maintenance_max_calls,
            retries: 2,
        },
    }))
}
