use super::*;
use nexofolio_common::UserId;
use nexofolio_contracts::endpoint::EndpointReader;
use nexofolio_contracts::knowledge::{
    CandidateActor, DocumentLifecycleWriter, LifecycleChange, LifecycleTarget,
};
fn actor() -> CandidateActor {
    CandidateActor::User {
        id: UserId::new(),
        display_name: Some("合成操作者".into()),
    }
}
#[tokio::test]
async fn retired_sources_do_not_invalidate_consumers_and_history_stays_followable() {
    let f = setup().await;
    let first = call(&f, "search_interfaces", json!({"query":"","limit":1}))
        .await
        .unwrap();
    f.knowledge
        .change_lifecycle(
            f.project,
            LifecycleChange {
                target: LifecycleTarget::Endpoint { id: f.related },
                retired: true,
                reason: "旧来源已被替代".into(),
                expected_version: 0,
            },
            actor(),
        )
        .await
        .unwrap();
    let active = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"overview"}),
    )
    .await
    .unwrap();
    assert_eq!(active["target_lifecycle"]["state"], "active");
    assert!(
        active["interface"]["retired_source_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(active["project_lifecycle"]["pending_reconciliation"], true);
    let current = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"relations"}),
    )
    .await
    .unwrap();
    assert_eq!(current["total"], 0);
    let old = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"relations","include_retired":true}),
    )
    .await
    .unwrap();
    assert!(old["total"].as_u64().unwrap() > 0);
    assert_eq!(old["items"][0]["lifecycle"]["state"], "retired_source");
    let evidence = call(
        &f,
        "read_source_evidence",
        old["items"][0]["evidence"]["arguments"].clone(),
    )
    .await
    .unwrap();
    let evidence: Value = serde_json::from_str(evidence["data"].as_str().unwrap()).unwrap();
    assert_eq!(evidence["lifecycle"]["state"], "retired_source");
    let exact = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.related,"section":"overview"}),
    )
    .await
    .unwrap();
    assert_eq!(exact["target_lifecycle"]["state"], "retired");
    assert_eq!(
        call(&f, "search_interfaces", first["next"]["arguments"].clone())
            .await
            .unwrap_err(),
        "SOURCE_CHANGED_RESTART_READ"
    );
    let all = call(
        &f,
        "search_interfaces",
        json!({"query":"","include_retired":true}),
    )
    .await
    .unwrap();
    assert_eq!(
        all["items"].as_array().unwrap().last().unwrap()["lifecycle"]["state"],
        "retired"
    );
}
#[tokio::test]
async fn exact_retired_field_and_raw_sample_reads_preserve_evidence_and_confirmation() {
    let f = setup().await;
    let facts = f.reader.get(f.target).await.unwrap().unwrap();
    let field = facts
        .fields
        .iter()
        .find(|v| v.path.to_string() == "customerId")
        .unwrap();
    let key = nexofolio_contracts::knowledge::field_note_key(field);
    let samples = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"samples"}),
    )
    .await
    .unwrap();
    let request = samples["items"][0]["request"]["arguments"].clone();
    let before = call(&f, "read_request_material", request.clone())
        .await
        .unwrap();
    f.knowledge
        .change_lifecycle(
            f.project,
            LifecycleChange {
                target: LifecycleTarget::Field {
                    endpoint: f.target,
                    location: field.location,
                    path: field.path.clone(),
                },
                retired: true,
                reason: "旧字段停用".into(),
                expected_version: 0,
            },
            actor(),
        )
        .await
        .unwrap();
    let read = call(
        &f,
        "read_interface",
        json!({"endpoint_id":f.target,"section":"field_summary","field_key":key}),
    )
    .await
    .unwrap();
    assert_eq!(read["target_lifecycle"]["state"], "retired");
    let source = call(
        &f,
        "read_source_evidence",
        json!({"endpoint_id":f.target,"field_key":key}),
    )
    .await
    .unwrap();
    let data: Value = serde_json::from_str(source["data"].as_str().unwrap()).unwrap();
    assert_eq!(data["retirement"]["reason"], "旧字段停用");
    let after = call(&f, "read_request_material", request).await.unwrap();
    assert_eq!(
        before["data"], after["data"],
        "lifecycle metadata must not modify original captured material"
    );
}
