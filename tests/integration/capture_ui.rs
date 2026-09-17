//! Real HTTP + isolated PostgreSQL contract tests, not installed-Chrome acceptance.
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
    let mut external: Value = serde_json::from_str(include_str!(
        "../../contracts/capture/fixtures/http-only.json"
    ))
    .unwrap();
    external["project_id"] = json!(project);
    external["source"]["instance_id"] = json!(Uuid::new_v4());
    let receipt = send(client, url, token, &external).await;
    assert_eq!(receipt["results"][0]["status"], "accepted", "{receipt}");
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    let context: Option<Value> =
        sqlx::query_scalar("SELECT context FROM capture_events WHERE id=$1")
            .bind(
                Uuid::parse_str(receipt["results"][0]["observation_id"].as_str().unwrap()).unwrap(),
            )
            .fetch_one(sql)
            .await
            .unwrap();
    assert!(
        context.is_none(),
        "non-browser sources do not fabricate page identities"
    );

    for case in [
        "ui-first",
        "http-first",
        "two-controls",
        "two-fields",
        "different-page",
        "different-frame",
        "different-view",
        "different-browser",
        "different-operation",
        "no-operation",
        "wrong-order",
        "expired",
        "different-producer",
        "different-environment",
    ] {
        let mut batch: Value = serde_json::from_str(include_str!(
            "../../contracts/capture/fixtures/query-form.json"
        ))
        .unwrap();
        batch["project_id"] = json!(project);
        batch["source"]["instance_id"] = json!(Uuid::new_v4());
        let mut ui = batch["records"][3].clone();
        let mut http = batch["records"][4].clone();
        for record in [&mut ui, &mut http] {
            record["record_id"] = json!(Uuid::new_v4());
        }
        let path = format!("/ui-sampling/{case}");
        http["payload"]["request"]["url"] = json!(format!("https://api.example.test{path}"));
        match case {
            "two-controls" => {
                let mut second = ui["payload"]["elements"][0].clone();
                second["element_id"] = json!("type");
                second["label"] = json!("类型");
                ui["payload"]["elements"]
                    .as_array_mut()
                    .unwrap()
                    .push(second);
            }
            "two-fields" => {
                http["payload"]["request"]["body"]["content"] = json!(r#"{"status":1,"type":1}"#)
            }
            "different-page" => http["context"]["page_instance_id"] = json!(Uuid::new_v4()),
            "different-frame" => http["context"]["frame_instance_id"] = json!(Uuid::new_v4()),
            "different-view" => http["context"]["view_id"] = json!(Uuid::new_v4()),
            "different-browser" => http["context"]["browser_instance_id"] = json!(Uuid::new_v4()),
            "different-operation" => http["context"]["interaction_id"] = json!(Uuid::new_v4()),
            "no-operation" => http["context"]["interaction_id"] = Value::Null,
            "wrong-order" => ui["captured_at"] = http["captured_at"].clone(),
            "expired" => ui["captured_at"] = json!("2026-09-16T09:57:00Z"),
            _ => {}
        }
        let late = case == "http-first";
        batch["records"] = json!([if late { &http } else { &ui }]);
        let receipt = send(client, url, token, &batch).await;
        assert_eq!(
            receipt["results"][0]["status"], "accepted",
            "{case}: {receipt}"
        );
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        if case == "different-producer" {
            batch["source"]["instance_id"] = json!(Uuid::new_v4());
        }
        if case == "different-environment" {
            batch["environment"] = json!({"name":"UI隔离环境"});
        }
        batch["records"] = json!([if late { &ui } else { &http }]);
        let receipt = send(client, url, token, &batch).await;
        assert_eq!(
            receipt["results"][0]["status"], "accepted",
            "{case}: {receipt}"
        );
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        let mappings: Vec<(Value, i64)> = sqlx::query_as("SELECT f.data,(SELECT count(*) FROM evidence_samples s WHERE s.fact_id=f.id) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path=$1 AND f.kind='enum_label_candidate'")
            .bind(&path).fetch_all(sql).await.unwrap();
        if matches!(case, "ui-first" | "http-first") {
            assert_eq!(mappings.len(), 1, "{case}");
            let (data, sources) = &mappings[0];
            assert_eq!(
                *sources, 2,
                "both HTTP and snapshot originals are traceable"
            );
            assert_eq!(data["label"], "待审核");
            assert_eq!(data["value"], 1);
            assert_eq!(data["ui_value"]["value"], "1");
            assert_eq!(data["transform"], "string_to_number");
            assert_eq!(data["verification"], "inferred");
            assert_eq!(data["complete_enum"], false);
        } else {
            assert!(
                mappings.is_empty(),
                "{case} must not create a field mapping: {mappings:?}"
            );
        }
        let before: i64 =
            sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
                .fetch_one(sql)
                .await
                .unwrap();
        let replay = send(client, url, token, &batch).await;
        assert_eq!(replay["results"][0]["replayed"], true);
        assert!(!capture.process_evidence_one().await.unwrap());
        let after: i64 = sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
            .fetch_one(sql)
            .await
            .unwrap();
        assert_eq!(before, after, "{case}: retry cannot add evidence support");
        if case == "ui-first" {
            // Reuse the proven control binding on another endpoint. Its absent field
            // is not an omission of the original interface's status parameter.
            let before: i64 = sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE kind='enum_label_candidate' AND data->>'state'='omitted'").fetch_one(sql).await.unwrap();
            let operation = Uuid::new_v4();
            for record in [&mut ui, &mut http] {
                record["record_id"] = json!(Uuid::new_v4());
                record["context"]["interaction_id"] = json!(operation);
            }
            ui["payload"]["elements"][0]["value"]["value"] = json!("");
            ui["payload"]["elements"][0]["options"][0]["value"]["value"] = json!("");
            ui["payload"]["elements"][0]["options"][0]["label"] = json!("全部");
            http["payload"]["request"]["url"] =
                json!("https://api.example.test/ui-sampling/other-endpoint");
            http["payload"]["request"]["body"]["content"] = json!("{}");
            batch["records"] = json!([ui, http]);
            send(client, url, token, &batch).await;
            while doc.process_one().await.unwrap().is_some() {}
            while capture.process_evidence_one().await.unwrap() {}
            let after: i64 = sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE kind='enum_label_candidate' AND data->>'state'='omitted'").fetch_one(sql).await.unwrap();
            assert_eq!(
                before, after,
                "a control binding cannot cross interface ownership"
            );
        }
    }
    // Independent A/B work on one page; B completes before A. Arrival order must
    // never substitute B's controls for the context frozen when A started.
    let mut interleaved: Value = serde_json::from_str(include_str!(
        "../../contracts/capture/fixtures/interleaved-use.json"
    ))
    .unwrap();
    interleaved["project_id"] = json!(project);
    let receipt = send(client, url, token, &interleaved).await;
    assert!(
        receipt["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "accepted")
    );
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    for (path, label, value) in [
        ("/orders/query", "待审核", 1),
        ("/inventory/search", "可用", 2),
    ] {
        let mappings: Vec<Value> = sqlx::query_scalar("SELECT f.data FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path=$1 AND f.kind='enum_label_candidate'")
            .bind(path).fetch_all(sql).await.unwrap();
        assert_eq!(mappings.len(), 1, "{path}");
        assert_eq!(mappings[0]["label"], label);
        assert_eq!(mappings[0]["value"], value);
        assert_eq!(mappings[0]["verification"], "inferred");
    }
    // Stage 2 regressions use separate endpoints and never touch recorded projects.
    {
        let mut dictionary: Value = serde_json::from_str(include_str!(
            "../../contracts/capture/fixtures/query-form.json"
        ))
        .unwrap();
        dictionary["project_id"] = json!(project);
        dictionary["source"]["instance_id"] = json!(Uuid::new_v4());
        let template = dictionary["records"][4].clone();
        let records: Vec<_> = [("north", "开放"), ("south", "关闭")]
            .into_iter()
            .map(|(region, label)| {
                let mut http = template.clone();
                http["record_id"] = json!(Uuid::new_v4());
                http["payload"]["request"]["url"] =
                    json!("https://api.example.test/stage2/dict/status");
                for (side, body) in [
                    ("request", json!({"region":region})),
                    ("response", json!({"items":[{"value":2,"label":label}]})),
                ] {
                    let content = body.to_string();
                    http["payload"][side]["body"]["bytes"] = json!(content.len());
                    http["payload"][side]["body"]["content"] = json!(content);
                }
                http
            })
            .collect();
        dictionary["records"] = json!(records);
        send(client, url, token, &dictionary).await;
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        let mappings:Vec<Value>=sqlx::query_scalar("SELECT f.data FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/stage2/dict/status' AND kind='dictionary_mapping_candidate'").fetch_all(sql).await.unwrap();
        assert_eq!(mappings.len(), 2);
        assert!(
            mappings.iter().all(|m| m["conflict"] != true),
            "different conditions cannot prove a conflict"
        );
        let mut contradictory = dictionary["records"][0].clone();
        contradictory["record_id"] = json!(Uuid::new_v4());
        let content = json!({"items":[{"value":2,"label":"同条件不同标签"}]}).to_string();
        contradictory["payload"]["response"]["body"]["bytes"] = json!(content.len());
        contradictory["payload"]["response"]["body"]["content"] = json!(content);
        dictionary["records"] = json!([contradictory]);
        send(client, url, token, &dictionary).await;
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        let conflicts:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/stage2/dict/status' AND f.data->>'conflict'='true'").fetch_one(sql).await.unwrap();
        assert_eq!(conflicts, 2, "only the two north mappings conflict");

        let mut partial: Value = serde_json::from_str(include_str!(
            "../../contracts/capture/fixtures/query-form.json"
        ))
        .unwrap();
        partial["project_id"] = json!(project);
        partial["source"]["instance_id"] = json!(Uuid::new_v4());
        let mut ui = partial["records"][3].clone();
        let mut http = partial["records"][4].clone();
        ui["record_id"] = json!(Uuid::new_v4());
        http["record_id"] = json!(Uuid::new_v4());
        http["payload"]["request"]["url"] = json!("https://api.example.test/stage2/partial");
        let content = json!({"a_status":1,"b_padding":vec![0;520],"z_type":1}).to_string();
        http["payload"]["request"]["body"]["bytes"] = json!(content.len());
        http["payload"]["request"]["body"]["content"] = json!(content);
        partial["records"] = json!([ui, http]);
        send(client, url, token, &partial).await;
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        let partial_bindings:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/stage2/partial' AND kind='enum_label_candidate'").fetch_one(sql).await.unwrap();
        assert_eq!(
            partial_bindings, 0,
            "a truncated request cannot prove a unique control binding"
        );
        let mut response_partial = partial.clone();
        for record in response_partial["records"].as_array_mut().unwrap() {
            record["record_id"] = json!(Uuid::new_v4());
        }
        let http = &mut response_partial["records"][1];
        http["payload"]["request"]["url"] =
            json!("https://api.example.test/stage2/response-partial");
        for (side, body) in [
            ("request", json!({"status":1})),
            ("response", json!({"padding":vec![0;520]})),
        ] {
            let text = body.to_string();
            http["payload"][side]["body"]["bytes"] = json!(text.len());
            http["payload"][side]["body"]["content"] = json!(text);
        }
        send(client, url, token, &response_partial).await;
        while doc.process_one().await.unwrap().is_some() {}
        while capture.process_evidence_one().await.unwrap() {}
        let safe_bindings:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/stage2/response-partial' AND kind='enum_label_candidate'").fetch_one(sql).await.unwrap();
        assert_eq!(
            safe_bindings, 1,
            "response truncation must not obscure complete request values"
        );
        if let Ok(report_path) = std::env::var("NEXOFOLIO_STAGE2_REPORT") {
            std::fs::write(report_path,serde_json::to_vec_pretty(&json!({
            "synthetic_cases":true,
            "filtered_dictionary_labels":mappings,
            "different_request_conditions_flagged_conflict":mappings.iter().all(|m|m["conflict"]==true),
            "partial_extraction_creates_unique_ui_mapping":partial_bindings>0,
            "partial_ui_mapping_count":partial_bindings,"same_scope_conflicting_mappings":conflicts,"safe_request_mappings_with_partial_response":safe_bindings
        })).unwrap()).unwrap();
        }
    }
}

// Optional cross-repository handoff: the plugin produces this synthetic batch by
// running its actual hook/manager/converter. Only the test project binding changes.
pub async fn verify_plugin_sample(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    project: ProjectId,
    sql: &sqlx::PgPool,
    doc: &ProcessingService,
    capture: &PostgresCaptureStore,
) {
    let Ok(path) = std::env::var("NEXOFOLIO_PLUGIN_SAMPLE_BATCH") else {
        return;
    };
    let mut batch: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    batch["project_id"] = json!(project);
    let receipt = send(client, url, token, &batch).await;
    let results = receipt["results"].as_array().unwrap();
    assert_eq!(results.len(), batch["records"].as_array().unwrap().len());
    assert!(
        results.iter().all(|r| r["status"] == "accepted"),
        "{receipt}"
    );
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    let (fact,): (Uuid,) = sqlx::query_as("SELECT f.id FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/api/orders' AND f.kind='enum_label_candidate' AND f.data->>'label'='待审核'")
        .fetch_one(sql).await.unwrap();
    let response = client
        .get(format!("{url}/v1/projects/{project}/evidence/{fact}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let fact: Value = response.json().await.unwrap();
    assert_eq!(fact["data"]["value"], 1);
    assert_eq!(fact["data"]["transform"], "string_to_number");
    assert_eq!(fact["data"]["verification"], "inferred");
    let unrelated: i64 = sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path IN ('/poll','/promise','/timer','/debounce') AND f.kind IN ('ui_field_label','enum_label_candidate')")
        .fetch_one(sql).await.unwrap();
    assert_eq!(unrelated, 0);
    if batch["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["payload"]["request"]["url"] == "https://sampling.invalid/api/b")
    {
        let labels: Vec<String> = sqlx::query_scalar("SELECT f.data->>'label' FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->>'interface_id' WHERE d.path='/api/b' AND f.kind='ui_field_label' AND f.subject->>'path'='/status'")
            .fetch_all(sql).await.unwrap();
        assert_eq!(
            labels,
            ["B状态"],
            "B cannot borrow A's order-status control"
        );
    }
    for result in results {
        let id = result["observation_id"].as_str().unwrap();
        let response = client
            .get(format!(
                "{url}/v1/projects/{project}/capture-observations/{id}"
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let observation: Value = response.json().await.unwrap();
        assert_eq!(observation["evidence_status"], "completed", "{observation}");
    }
    let replay = send(client, url, token, &batch).await;
    assert!(
        replay["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["replayed"] == true)
    );
    assert!(!capture.process_evidence_one().await.unwrap());
    println!(
        "plugin_generated_batch_verified: {} records, enum mapping, polling isolation, HTTP evidence readback and retry; synthetic browser only",
        results.len()
    );
}
