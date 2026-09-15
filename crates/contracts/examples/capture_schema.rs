fn main() {
    use nexofolio_contracts::*;
    println!(
        "{}",
        serde_json::json!({
         "capabilities.schema.json":schemars::schema_for!(CaptureCapabilities),"batch.schema.json":schemars::schema_for!(CaptureBatch),"record.schema.json":schemars::schema_for!(CaptureRecord),"context.schema.json":schemars::schema_for!(CaptureContext),
         "page-context.schema.json":schemars::schema_for!(PageContextPayload),"interaction.schema.json":schemars::schema_for!(InteractionPayload),"ui-snapshot.schema.json":schemars::schema_for!(UiSnapshotPayload),"image-reference.schema.json":schemars::schema_for!(ImageReferencePayload),
         "receipt.schema.json":schemars::schema_for!(CaptureBatchReceipt),"asset.schema.json":schemars::schema_for!(AssetReceipt),"observation.schema.json":schemars::schema_for!(CaptureObservation),"evidence.schema.json":schemars::schema_for!(EvidenceFact),"evidence-page.schema.json":schemars::schema_for!(EvidencePage)
        })
    );
}
