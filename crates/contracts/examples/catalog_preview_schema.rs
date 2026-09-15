fn main() {
    use nexofolio_contracts::{
        CatalogActivation, CatalogVersionPage, OfficialCatalog, OfficialInterfacePage,
        PublishCatalog, RestoreCatalog,
    };
    use nexofolio_contracts::{
        CatalogPreviewDetail, CatalogPreviewPage, CatalogSnapshot, DirectoryCandidate, PreviewTask,
    };
    println!(
        "{}",
        serde_json::json!({
            "candidate.schema.json":schemars::schema_for!(DirectoryCandidate),
            "snapshot.schema.json":schemars::schema_for!(CatalogSnapshot),
            "task.schema.json":schemars::schema_for!(PreviewTask),
            "page.schema.json":schemars::schema_for!(CatalogPreviewPage),
            "detail.schema.json":schemars::schema_for!(CatalogPreviewDetail),
            "official.schema.json":schemars::schema_for!(OfficialCatalog),
            "official-interfaces.schema.json":schemars::schema_for!(OfficialInterfacePage),
            "versions.schema.json":schemars::schema_for!(CatalogVersionPage),
            "publish.schema.json":schemars::schema_for!(PublishCatalog),
            "restore.schema.json":schemars::schema_for!(RestoreCatalog),
            "activation.schema.json":schemars::schema_for!(CatalogActivation)
        })
    );
}
