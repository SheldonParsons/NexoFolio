//! Arrival-order and overload regressions using the existing isolated capture harness.
use super::*;
pub async fn verify(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    project: ProjectId,
    sql: &sqlx::PgPool,
    doc: &ProcessingService,
    capture: &PostgresCaptureStore,
) {
    let mut reports = vec![];
    for order in ["forward", "reverse"] {
        let mut batch: Value = serde_json::from_str(include_str!(
            "../../contracts/capture/fixtures/query-form.json"
        ))
        .unwrap();
        batch["project_id"] = json!(project);
        batch["source"]["instance_id"] = json!(Uuid::new_v4());
        let template = batch["records"][4].clone();
        let actions = [Uuid::new_v4(), Uuid::new_v4()];
        let records: Vec<_> = [
            ("source", 23),
            ("target", 23),
            ("source", 24),
            ("target", 25),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (side, value))| {
            let mut r = template.clone();
            r["record_id"] = json!(Uuid::new_v4());
            let start = 1_789_552_805_001_i64 + i as i64 * 1000;
            r["context"]["request_started_at_ms"] = json!(start);
            r["context"]["response_completed_at_ms"] = json!(start + 100);
            r["context"]["interaction_id"] = json!(actions[i / 2]);
            r["context"]["event_seq"] = json!(i + 1);
            r["captured_at"] = json!(format!("2026-09-16T10:00:0{}.101Z", 5 + i));
            r["payload"]["request"]["url"] = json!(format!(
                "https://api.example.test/order-regression/{order}/{side}"
            ));
            for (part, body) in [
                (
                    "request",
                    if side == "target" {
                        json!({"record_id":value})
                    } else {
                        json!({})
                    },
                ),
                (
                    "response",
                    if side == "source" {
                        json!({"id":value})
                    } else {
                        json!({"ok":true})
                    },
                ),
            ] {
                let s = body.to_string();
                r["payload"][part]["body"]["bytes"] = json!(s.len());
                r["payload"][part]["body"]["content"] = json!(s);
            }
            r
        })
        .collect();
        batch["records"] = json!(records);
        let receipt = send(client, url, token, &batch).await;
        while doc.process_one().await.unwrap().is_some() {}
        let ids: Vec<Uuid> = receipt["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["observation_id"].as_str().unwrap().parse().unwrap())
            .collect();
        for (position, index) in if order == "forward" {
            vec![0, 1, 2, 3]
        } else {
            vec![3, 2, 1, 0]
        }
        .into_iter()
        .enumerate()
        {
            sqlx::query("UPDATE capture_events SET received_at=to_timestamp($2::double precision) WHERE id=$1").bind(ids[index]).bind(1_000_000.0+position as f64).execute(sql).await.unwrap();
        }
        while capture.process_evidence_one().await.unwrap() {}
        let r:Value=sqlx::query_scalar("SELECT jsonb_build_object('candidates',count(*) FILTER(WHERE f.kind='parameter_link_candidate'),'counterexamples',count(*) FILTER(WHERE f.kind='parameter_link_counterexample'),'counter_support',sum(f.observations) FILTER(WHERE f.kind='parameter_link_counterexample'),'conflicted',count(*) FILTER(WHERE f.kind='parameter_link_candidate' AND f.data->>'conflict'='true')) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->'target'->>'interface_id' WHERE d.path=$1").bind(format!("/order-regression/{order}/target")).fetch_one(sql).await.unwrap();
        assert_eq!(r["candidates"], 1);
        assert_eq!(r["counterexamples"], 1);
        assert_eq!(r["counter_support"], 1);
        assert_eq!(r["conflicted"], 1);
        reports.push(r);
    }
    assert_eq!(
        reports[0], reports[1],
        "counterexamples must not depend on arrival order"
    );
    // More than the per-event pair budget is a visible failure, never a committed prefix.
    let mut batch: Value = serde_json::from_str(include_str!(
        "../../contracts/capture/fixtures/query-form.json"
    ))
    .unwrap();
    batch["project_id"] = json!(project);
    batch["source"]["instance_id"] = json!(Uuid::new_v4());
    let template = batch["records"][4].clone();
    let wide: serde_json::Map<String, Value> = (0..260)
        .map(|i| (format!("item_{i:03}"), json!(23)))
        .collect();
    let records: Vec<_> = [
        ("source", json!({}), Value::Object(wide)),
        ("target", json!({"ref_a":23,"ref_b":23}), json!({"ok":true})),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (side, req, res))| {
        let mut r = template.clone();
        r["record_id"] = json!(Uuid::new_v4());
        r["payload"]["request"]["url"] =
            json!(format!("https://api.example.test/pair-budget/{side}"));
        r["captured_at"] = json!(format!("2026-09-16T10:00:0{}.101Z", 5 + i));
        r["context"]["request_started_at_ms"] = json!(1_789_552_805_001_i64 + i as i64 * 1000);
        r["context"]["response_completed_at_ms"] = json!(1_789_552_805_101_i64 + i as i64 * 1000);
        for (part, body) in [("request", req), ("response", res)] {
            let text = body.to_string();
            r["payload"][part]["body"]["bytes"] = json!(text.len());
            r["payload"][part]["body"]["content"] = json!(text);
        }
        r
    })
    .collect();
    batch["records"] = json!(records);
    let receipt = send(client, url, token, &batch).await;
    while doc.process_one().await.unwrap().is_some() {}
    let ids: Vec<Uuid> = receipt["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["observation_id"].as_str().unwrap().parse().unwrap())
        .collect();
    for (i, id) in ids.iter().enumerate() {
        sqlx::query(
            "UPDATE capture_events SET received_at=to_timestamp($2::double precision) WHERE id=$1",
        )
        .bind(id)
        .bind(1_000_000.0 + i as f64)
        .execute(sql)
        .await
        .unwrap();
    }
    assert!(capture.process_evidence_one().await.unwrap());
    for _ in 0..5 {
        sqlx::query("UPDATE capture_events SET retry_at=clock_timestamp() WHERE id=$1")
            .bind(ids[1])
            .execute(sql)
            .await
            .unwrap();
        assert!(matches!(
            capture.process_evidence_one().await,
            Err(Error::Unavailable {
                component: "evidence_relation_budget"
            })
        ));
    }
    let state: (String, String, bool) = sqlx::query_as(
        "SELECT evidence_status,error_code,raw_hash IS NOT NULL FROM capture_events WHERE id=$1",
    )
    .bind(ids[1])
    .fetch_one(sql)
    .await
    .unwrap();
    assert_eq!(
        state,
        ("failed".into(), "RELATION_SEARCH_LIMIT".into(), true)
    );
    let indexed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM evidence_value_index WHERE event_id=$1")
            .bind(ids[1])
            .fetch_one(sql)
            .await
            .unwrap();
    assert_eq!(
        indexed, 0,
        "the failed event transaction must leave no partial index or links"
    );
}

#[test]
fn evidence_field_contract_requires_one_basis() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../contracts/capture/evidence-field.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let field = json!({"interface_id":Uuid::new_v4(),"environment_id":Uuid::new_v4(),"revision_id":Uuid::new_v4(),"location":"response.body","path":"/data/id"});
    assert!(validator.is_valid(&field));
    let mut observed = field.clone();
    observed.as_object_mut().unwrap().remove("revision_id");
    assert!(!validator.is_valid(&observed));
    observed["observation"] = json!({"project_id":Uuid::new_v4(),"ingestion_id":Uuid::new_v4(),"interface_id":field["interface_id"],"environment_id":field["environment_id"],"location":field["location"],"path":field["path"]});
    assert!(validator.is_valid(&observed));
    observed["revision_id"] = field["revision_id"].clone();
    assert!(!validator.is_valid(&observed));
    observed["revision_id"] = Value::Null;
    assert!(!validator.is_valid(&observed));
}
