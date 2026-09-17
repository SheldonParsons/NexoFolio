//! Offline verification of the production review transport, using a fixed private snapshot.
use nexofolio_contracts::KnowledgeSnapshot;
use nexofolio_rebuild::ReviewSegment;
use serde_json::json;
fn main() {
    let snapshot: KnowledgeSnapshot = serde_json::from_slice(
        &std::fs::read(std::env::args().nth(1).expect("snapshot file")).unwrap(),
    )
    .unwrap();
    let model = nexofolio_infrastructure::ChatMaintenanceModel::new(
        "http://127.0.0.1:9/v1",
        nexofolio_contracts::Secret::new("unused-offline"),
        "deepseek-v4-pro".into(),
        false,
    )
    .unwrap();
    let (prepared, estimate) =
        nexofolio_application::prepare_maintenance_review(&snapshot, &model, &Default::default())
            .unwrap();
    println!("{}", serde_json::to_string(&estimate).unwrap());
    for segments in [prepared] {
        let limit = estimate.input_limit_bytes;
        let mut bytes = 0;
        let mut fields = std::collections::HashSet::new();
        for segment in &segments {
            let payload = segment.model_input();
            let size = serde_json::to_vec(&payload).unwrap().len();
            assert!(size <= limit);
            bytes += size;
            let expanded = ReviewSegment::from_model_input(payload).unwrap();
            assert_eq!(
                serde_json::to_value(segment).unwrap(),
                serde_json::to_value(expanded).unwrap()
            );
            fields.extend(segment.units.iter().map(|u| u.field_id.clone()));
        }
        assert_eq!(fields.len(), snapshot.fields.len());
        println!(
            "{}",
            json!({"limit":limit,"segments":segments.len(),"fields":fields.len(),"payload_bytes":bytes,"round_trip_equal":true})
        );
    }
}
