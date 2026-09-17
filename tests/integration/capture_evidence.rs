mod capture_benchmark;
mod capture_rollback;
mod capture_ui;
mod evidence_order;
mod maintenance_fixture;
mod snapshot_inputs;
use nexofolio_access::{
    ExternalIdentity, ExternalProject, PlatformAccess, ProjectSnapshot, SessionPrincipal,
};
use nexofolio_application::ProcessingService;
use nexofolio_backend::{
    http,
    wiring::{Config, build_access},
};
use nexofolio_contracts::*;
use nexofolio_infrastructure::{
    FileBlobStore, Postgres, PostgresAccess, PostgresCaptureStore, PostgresDocuments, Unconfigured,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
async fn send(client: &reqwest::Client, url: &str, token: &str, batch: &Value) -> Value {
    let response = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(batch)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn duplicate_structure_keeps_evidence_and_relation_context() {
    let original = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = sqlx::PgPool::connect(&original).await.unwrap();
    let schema = format!("capture_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let sep = if original.contains('?') { "&" } else { "?" };
    let db_url = format!("{original}{sep}options=-csearch_path%3D{schema}");
    let db = Postgres::new(&Secret::new(&db_url), 10, Duration::from_secs(2)).unwrap();
    db.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&db_url).await.unwrap();
    let blobs = std::env::temp_dir().join(format!("nexo-capture-{}", Uuid::new_v4()));
    let key = "c8".repeat(32);
    let instance = "http://127.0.0.1:1/capture";
    let store = PostgresAccess::new(db.clone(), &Secret::new(&key)).unwrap();
    let session = store
        .normal_login(&ExternalIdentity {
            instance: instance.into(),
            external_id: "u".into(),
            account: "u".into(),
            display_name: "Capture".into(),
        })
        .await
        .unwrap();
    let principal = SessionPrincipal {
        user_id: session.user.id,
        instance: instance.into(),
    };
    store
        .apply_snapshot(
            &principal,
            store.next_sync_generation().await.unwrap(),
            ProjectSnapshot {
                projects: vec![
                    ExternalProject {
                        instance: instance.into(),
                        external_id: "p".into(),
                        name: "Capture fixture".into(),
                        state: "doing".into(),
                    },
                    ExternalProject {
                        instance: instance.into(),
                        external_id: "denied".into(),
                        name: "Denied".into(),
                        state: "doing".into(),
                    },
                ],
                visible_project_ids: vec!["p".into()],
            },
        )
        .await
        .unwrap();
    let projects = store.list_projects(&principal, 1, 100).await.unwrap();
    let p = projects
        .items
        .iter()
        .find(|p| p.can_access)
        .unwrap()
        .project_id;
    let denied = projects
        .items
        .iter()
        .find(|p| !p.can_access)
        .unwrap()
        .project_id;
    let config = Config::from_lookup(|name| match name {
        "DATABASE_URL" => Some(db_url.clone()),
        "NEXOFOLIO_ZENTAO_BASE_URL" => Some(instance.into()),
        "NEXOFOLIO_SESSION_KEY" => Some(key.clone()),
        "NEXOFOLIO_CAPTURE_ENABLED" => Some("true".into()),
        "NEXOFOLIO_BLOB_ROOT" => Some(blobs.to_string_lossy().into()),
        _ => None,
    })
    .unwrap();
    let stop = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = http::router_with_access(
        &config,
        Arc::new(db.clone()),
        Arc::new(Unconfigured),
        stop.clone(),
        build_access(&config, db.clone()).unwrap(),
    );
    let server = tokio::spawn(http::serve(
        listener,
        app,
        stop.clone(),
        Duration::from_secs(2),
    ));
    let client = reqwest::Client::new();
    let token = session.token.expose();
    let legacy_caps: Value = client
        .get(format!("{url}/v1/ingestion/capabilities"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(legacy_caps["schema_versions"], json!(["1", "2"]));
    let caps: Value = client
        .get(format!("{url}/v1/ingestion/capabilities?schema_version=3"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps["schema_versions"], json!(["1", "2", "3"]));
    let mut batch: Value = serde_json::from_str(include_str!(
        "../../contracts/ingestion/fixtures/http-batch.json"
    ))
    .unwrap();
    batch["schema_version"] = json!("3");
    batch["project_id"] = json!(p);
    batch["source"]["instance_id"] = json!(Uuid::new_v4());
    let template = batch["records"][0].clone();
    let browser = Uuid::new_v4();
    let page = Uuid::new_v4();
    let frame = Uuid::new_v4();
    let view = Uuid::new_v4();
    let interaction = Uuid::new_v4();
    let at = 1_789_382_400_000i64;
    let context = |seq, start, end, action: Option<Uuid>| json!({"browser_instance_id":browser,"page_instance_id":page,"frame_instance_id":frame,"view_id":view,"event_seq":seq,"page_url":"https://portal.example.test/orders","interaction_id":action,"request_started_at_ms":start,"response_completed_at_ms":end});
    let http_record = |path: &str,
                       body: Value,
                       request: Option<Value>,
                       seq: u64,
                       start: i64,
                       end: i64,
                       action: Option<Uuid>| {
        let mut r = template.clone();
        r["record_id"] = json!(Uuid::new_v4());
        r["context"] = context(seq, Some(start), Some(end), action);
        r["payload"]["request"]["url"] = json!(format!("https://api.example.test{path}"));
        r["payload"]["response"]["body"]["content"] = json!(body.to_string());
        if let Some(request) = request {
            r["payload"]["request"]["method"] = json!("POST");
            r["payload"]["request"]["headers"]["entries"] =
                json!([["Content-Type", "application/json"]]);
            r["payload"]["request"]["body"] = json!({"state":"complete","encoding":"text","content":request.to_string(),"bytes":request.to_string().len()});
        }
        r
    };
    let ui = |value: &str, label: &str, seq: u64, action: Uuid| json!({"record_id":Uuid::new_v4(),"kind":"interaction","payload_version":"1","captured_at":sqlx::types::chrono::DateTime::from_timestamp_millis(at+1500).unwrap().to_rfc3339(),"context":context(seq,None::<i64>,None::<i64>,Some(action)),"payload":{"action":"change","target":{"element_id":"status-select","tag":"select","role":"combobox","name":"订单状态","label":"订单状态","value":{"state":"present","value":value},"options":[{"label":label,"value":{"state":"present","value":value},"selected":true}],"bounds":null,"visible":true},"complete":true,"limitations":[]}});
    let a = http_record(
        "/orders",
        json!({"items":[{"id":23,"status":1}]}),
        None,
        1,
        at,
        at + 1000,
        None,
    );
    batch["records"] = json!([
        a,
        ui("1", "待审核", 2, interaction),
        http_record(
            "/details",
            json!({"ok":true}),
            Some(json!({"record_id":"23","status":1})),
            3,
            at + 2000,
            at + 3000,
            Some(interaction)
        )
    ]);
    let first = send(&client, &url, token, &batch).await;
    assert!(
        first["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "accepted")
    );
    let doc = ProcessingService::new(Arc::new(PostgresDocuments::new(db.clone())));
    while doc.process_one().await.unwrap().is_some() {}
    let capture =
        PostgresCaptureStore::new(db.clone(), Arc::new(FileBlobStore::new(blobs.clone())));
    for _ in 0..10 {
        if !capture.process_evidence_one().await.unwrap() {
            break;
        }
    }
    let next_action = Uuid::new_v4();
    batch["records"] = json!([
        ui("2", "已通过", 4, next_action),
        http_record(
            "/details",
            json!({"ok":true}),
            Some(json!({"record_id":"23","status":2})),
            5,
            at + 4000,
            at + 5000,
            Some(next_action)
        )
    ]);
    let second = send(&client, &url, token, &batch).await;
    assert_eq!(second["results"][1]["structure"], "duplicate");
    assert_eq!(second["results"][1]["status"], "accepted");
    assert!(doc.process_one().await.unwrap().is_none());
    for _ in 0..10 {
        if !capture.process_evidence_one().await.unwrap() {
            break;
        }
    }
    let facts = capture
        .facts(session.user.id, p, 1, 100, None)
        .await
        .unwrap();
    assert!(
        facts
            .items
            .iter()
            .any(|f| f.kind == "parameter_link_candidate"
                && f.subject["source"]["path"] == "/items/*/id"
                && f.subject["target"]["path"] == "/record_id")
    );
    assert!(facts.items.iter().any(|f| f.kind == "enum_label_candidate"
        && f.data["value"] == 2
        && f.data["label"] == "已通过"));
    let before: i64 = sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
        .fetch_one(&sql)
        .await
        .unwrap();
    let replay_batch = batch.clone();
    let replay = send(&client, &url, token, &batch).await;
    assert!(
        replay["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["replayed"] == true)
    );
    assert!(!capture.process_evidence_one().await.unwrap());
    let after: i64 = sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(before, after);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM interface_documents")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let event: Uuid = second["results"][1]["observation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let observation = capture
        .observation(session.user.id, p, event)
        .await
        .unwrap();
    assert_eq!(
        observation.payload.unwrap()["request"]["body"]["content"],
        batch["records"][1]["payload"]["request"]["body"]["content"]
    );
    assert_eq!(
        client
            .get(format!(
                "{url}/v1/projects/{denied}/capture-observations/{event}"
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let asset = Uuid::new_v4();
    let png = include_bytes!("../fixtures/pixel.png").to_vec();
    let response = client
        .put(format!("{url}/v1/projects/{p}/assets/{asset}"))
        .bearer_auth(token)
        .header("Content-Type", "image/png")
        .body(png.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = client
        .get(format!("{url}/v1/projects/{p}/assets/{asset}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), png.as_slice());
    let reply: Value = client
        .put(format!("{url}/v1/projects/{p}/assets/{asset}"))
        .bearer_auth(token)
        .header("Content-Type", "image/png")
        .body(png)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reply["replayed"], true);
    assert_eq!(
        client
            .get(format!("{url}/v1/projects/{denied}/assets/{asset}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );

    if std::env::var("NEXOFOLIO_KEEP_CAPTURE_FIXTURE").is_err() {
        // Same-page SPA bridge is weak evidence; a different page or impossible timing is not a link.
        let bridge_action = Uuid::new_v4();
        let mut click = ui("23", "打开订单", 6, bridge_action);
        click["payload"]["target"]["element_id"] = json!("order-row");
        let mut bridge = http_record(
            "/bridge/23",
            json!({"ok":true}),
            None,
            7,
            at + 10000,
            at + 11000,
            Some(bridge_action),
        );
        bridge["context"]["view_id"] = json!(Uuid::new_v4());
        let mut unrelated = http_record(
            "/unrelated",
            json!({"ok":true}),
            Some(json!({"record_id":"23"})),
            8,
            at + 10000,
            at + 11000,
            None,
        );
        unrelated["context"]["page_instance_id"] = json!(Uuid::new_v4());
        let early = http_record(
            "/early",
            json!({"ok":true}),
            Some(json!({"record_id":"23"})),
            9,
            at + 100,
            at + 500,
            None,
        );
        let common = http_record(
            "/common",
            json!({"ok":true}),
            Some(json!({"one":1,"zero":0})),
            10,
            at + 10000,
            at + 11000,
            None,
        );
        batch["records"] = json!([bridge, unrelated, early, common]);
        send(&client, &url, token, &batch).await;
        while doc.process_one().await.unwrap().is_some() {}
        for _ in 0..20 {
            if !capture.process_evidence_one().await.unwrap() {
                break;
            }
        }
        let before_bridge:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN interface_documents d ON d.id::text=f.subject->'target'->>'interface_id' WHERE f.kind='parameter_link_candidate' AND d.path='/bridge/{param1}'").fetch_one(&sql).await.unwrap();
        assert_eq!(
            before_bridge, 0,
            "different views require an actual captured bridge"
        );
        batch["records"] = json!([click]);
        send(&client, &url, token, &batch).await;
        while capture.process_evidence_one().await.unwrap() {}
        let links: Vec<Value> = sqlx::query_scalar(
            "SELECT subject FROM evidence_facts WHERE kind='parameter_link_candidate'",
        )
        .fetch_all(&sql)
        .await
        .unwrap();
        let by_path: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id,path FROM interface_documents")
                .fetch_all(&sql)
                .await
                .unwrap();
        for path in ["/unrelated", "/early", "/common"] {
            let id = by_path.iter().find(|(_, p)| p == path).unwrap().0;
            assert!(
                !links
                    .iter()
                    .any(|l| l["target"]["interface_id"] == id.to_string()),
                "unexpected link to {path}"
            );
        }
        let bridge_id = by_path
            .iter()
            .find(|(_, p)| p == "/bridge/{param1}")
            .unwrap()
            .0;
        assert!(links.iter().any(|l| l["source"]["path"] == "/items/*/id"
            && l["target"]["interface_id"] == bridge_id.to_string()));
        // A prior control/field binding plus a complete request can establish omission, even for a blank UI value.
        let omitted_action = Uuid::new_v4();
        batch["records"] = json!([
            ui("", "全部", 11, omitted_action),
            http_record(
                "/details",
                json!({"ok":true}),
                Some(json!({"record_id":"23"})),
                12,
                at + 12000,
                at + 13000,
                Some(omitted_action)
            )
        ]);
        send(&client, &url, token, &batch).await;
        while doc.process_one().await.unwrap().is_some() {}
        for _ in 0..10 {
            if !capture.process_evidence_one().await.unwrap() {
                break;
            }
        }
        let omitted:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE kind='enum_label_candidate' AND data->>'state'='omitted' AND data->'value'='null'::jsonb AND data->>'label'='全部'").fetch_one(&sql).await.unwrap();
        assert_eq!(omitted, 1);
        // Conflicting labels stay visible and never become a complete enum constraint.
        let conflict_action = Uuid::new_v4();
        batch["records"] = json!([
            ui("2", "已拒绝", 13, conflict_action),
            http_record(
                "/details",
                json!({"ok":true}),
                Some(json!({"record_id":"23","status":2})),
                14,
                at + 14000,
                at + 15000,
                Some(conflict_action)
            )
        ]);
        send(&client, &url, token, &batch).await;
        while doc.process_one().await.unwrap().is_some() {}
        for _ in 0..10 {
            if !capture.process_evidence_one().await.unwrap() {
                break;
            }
        }
        let conflicts:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE kind='enum_label_candidate' AND data->>'conflict'='true'").fetch_one(&sql).await.unwrap();
        assert_eq!(conflicts, 2);
        let original_environment = batch["environment"].clone();
        batch["environment"] = json!({"name":"隔离环境"});
        batch["records"] = json!([http_record(
            "/cross-environment",
            json!({"ok":true}),
            Some(json!({"record_id":"23"})),
            15,
            at + 16000,
            at + 17000,
            None
        )]);
        send(&client, &url, token, &batch).await;
        while doc.process_one().await.unwrap().is_some() {}
        for _ in 0..10 {
            if !capture.process_evidence_one().await.unwrap() {
                break;
            }
        }
        let cross:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts f JOIN environments e ON e.id=f.environment_id WHERE f.kind='parameter_link_candidate' AND e.name='隔离环境'").fetch_one(&sql).await.unwrap();
        assert_eq!(cross, 0);
        batch["environment"] = original_environment;
        maintenance_fixture::verify(
            &db,
            Arc::new(capture.clone()),
            session.user.id,
            p,
            denied,
            &sql,
        )
        .await;
    }

    if std::env::var("NEXOFOLIO_KEEP_CAPTURE_FIXTURE").is_err() {
        capture_ui::verify(&client, &url, token, p, &sql, &doc, &capture).await;
        capture_ui::verify_plugin_sample(&client, &url, token, p, &sql, &doc, &capture).await;
        capture_rollback::verify(&db, &sql, &principal, &replay_batch).await;
        let pending: i64 =
            sqlx::query_scalar("SELECT pending FROM capture_backlog WHERE project_id=$1")
                .bind(Uuid::parse_str(&p.to_string()).unwrap())
                .fetch_one(&sql)
                .await
                .unwrap();
        sqlx::query("UPDATE capture_backlog SET pending=50000 WHERE project_id=$1")
            .bind(Uuid::parse_str(&p.to_string()).unwrap())
            .execute(&sql)
            .await
            .unwrap();
        let replay = send(&client, &url, token, &replay_batch).await;
        assert!(
            replay["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["replayed"] == true)
        );
        let mut overloaded = replay_batch.clone();
        overloaded["records"] = json!([overloaded["records"][1].clone()]);
        overloaded["records"][0]["record_id"] = json!(Uuid::new_v4());
        let response = client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&overloaded)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers()["retry-after"], "2");
        sqlx::query("UPDATE capture_backlog SET pending=$2 WHERE project_id=$1")
            .bind(Uuid::parse_str(&p.to_string()).unwrap())
            .bind(pending)
            .execute(&sql)
            .await
            .unwrap();
    }

    if std::env::var("NEXOFOLIO_KEEP_CAPTURE_FIXTURE").is_err() {
        let mut repeated = replay_batch.clone();
        let mut records = Vec::new();
        for n in 0..9 {
            let mut r = repeated["records"][1].clone();
            r["record_id"] = json!(Uuid::new_v4());
            r.as_object_mut().unwrap().remove("context");
            r["captured_at"] = json!(
                sqlx::types::chrono::DateTime::from_timestamp_millis(at + 100000 + n * 1000)
                    .unwrap()
                    .to_rfc3339()
            );
            records.push(r);
        }
        repeated["records"] = json!(records);
        let receipts = send(&client, &url, token, &repeated).await;
        while doc.process_one().await.unwrap().is_some() {}
        for _ in 0..20 {
            if !capture.process_evidence_one().await.unwrap() {
                break;
            }
        }
        let largest:Option<i64>=sqlx::query_scalar("SELECT max(n) FROM(SELECT count(*) n FROM evidence_sample_groups GROUP BY fact_id) groups").fetch_one(&sql).await.unwrap();
        assert!(largest.unwrap() <= 5);
        sqlx::query("UPDATE capture_events SET received_at=clock_timestamp()-interval '25 hours'")
            .execute(&sql)
            .await
            .unwrap();
        // A competing cleaner must be skipped instead of blocking capture shutdown.
        let mut cleaner = sql.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('evidence-retention',0))")
            .execute(&mut *cleaner)
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), capture.collect_expired_evidence())
                .await
                .unwrap()
                .unwrap(),
            0
        );
        cleaner.rollback().await.unwrap();
        assert!(capture.collect_expired_evidence().await.unwrap() > 0);
        let lost_pins:i64=sqlx::query_scalar("SELECT count(*) FROM evidence_pins p JOIN capture_events e ON e.id=p.event_id WHERE e.raw_hash IS NULL").fetch_one(&sql).await.unwrap();
        assert_eq!(lost_pins, 0);
        let mut unavailable = 0;
        for row in receipts["results"].as_array().unwrap() {
            let id: Uuid = row["observation_id"].as_str().unwrap().parse().unwrap();
            let observation = capture.observation(session.user.id, p, id).await.unwrap();
            if !observation.payload_available {
                assert!(observation.payload.is_none());
                unavailable += 1;
            }
        }
        assert!(
            unavailable > 0,
            "unreferenced repeats expire while pinned originals survive"
        );
        let lost_definitions:i64=sqlx::query_scalar("SELECT count(*) FROM interface_observed_revisions r JOIN ingestion_inbox i ON i.id=r.origin_ingestion_id WHERE i.raw_record IS NULL").fetch_one(&sql).await.unwrap();
        assert_eq!(lost_definitions, 0);
    }
    snapshot_inputs::verify(&db, &sql, &capture, session.user.id, p).await;
    evidence_order::verify(&client, &url, token, p, &sql, &doc, &capture).await;
    capture_benchmark::compare(&db_url, instance, &key, token, &blobs, &replay_batch).await;
    // Ordinary values are bounded; dictionary mappings and relation indexes are not discarded.
    let mut bounded = batch.clone();
    bounded["batch_id"] = json!(Uuid::new_v4());
    bounded["records"] = json!([http_record(
        "/options/retention-fixture",
        json!({"items":(9000..9040).map(|id|json!({"id":id,"name":format!("Option {id}")})).collect::<Vec<_>>()}),
        None,
        9000,
        at + 900000,
        at + 900100,
        None,
    )]);
    let received = send(&client, &url, token, &bounded).await;
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    let event: Uuid = received["results"][0]["observation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT o.interface_id FROM capture_events e JOIN interface_observations o ON o.ingestion_id=e.ingestion_id WHERE e.id=$1").bind(event).fetch_one(&sql).await.unwrap();
    let count = |kind: &'static str| {
        let sql = &sql;
        async move {
            sqlx::query_scalar::<_,i64>("SELECT count(*) FROM evidence_facts WHERE subject->>'interface_id'=$1 AND subject->>'path'='/items/*/id' AND kind=$2").bind(owner.to_string()).bind(kind).fetch_one(sql).await.unwrap()
        }
    };
    assert_eq!(
        count("observed_value").await,
        nexofolio_evidence::OBSERVED_VALUE_LIMIT
    );
    assert_eq!(count("observed_value_limit").await, 1);
    assert_eq!(count("dictionary_mapping_candidate").await, 40);
    let indexed: i64 = sqlx::query_scalar("SELECT count(*) FROM evidence_value_index WHERE event_id=$1 AND field_ref->>'path'='/items/*/id'").bind(event).fetch_one(&sql).await.unwrap();
    assert_eq!(indexed, 40);
    let before: i64 = sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
        .fetch_one(&sql)
        .await
        .unwrap();
    let retry = send(&client, &url, token, &bounded).await;
    assert_eq!(retry["results"][0]["replayed"], true);
    assert!(!capture.process_evidence_one().await.unwrap());
    let after: i64 = sqlx::query_scalar("SELECT sum(observations)::bigint FROM evidence_facts")
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(before, after);
    bounded["batch_id"] = json!(Uuid::new_v4());
    bounded["records"] = json!([http_record(
        "/options/retention-fixture",
        json!({"items":std::iter::once(9000).chain(9040..9080).map(|id|json!({"id":id,"name":format!("Option {id}")})).collect::<Vec<_>>()}),
        None,
        9002,
        at + 900110,
        at + 900150,
        None
    )]);
    let more = send(&client, &url, token, &bounded).await;
    assert_eq!(more["results"][0]["structure"], "duplicate");
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    assert_eq!(
        count("observed_value").await,
        nexofolio_evidence::OBSERVED_VALUE_LIMIT
    );
    assert_eq!(count("observed_value_limit").await, 1);
    assert_eq!(count("dictionary_mapping_candidate").await, 80);
    let support:i64=sqlx::query_scalar("SELECT observations FROM evidence_facts WHERE subject->>'interface_id'=$1 AND subject->>'path'='/items/*/id' AND kind='observed_value' AND data->'value'='9000'::jsonb").bind(owner.to_string()).fetch_one(&sql).await.unwrap();
    assert_eq!(
        support, 2,
        "known samples keep accumulating after saturation"
    );
    bounded["batch_id"] = json!(Uuid::new_v4());
    bounded["records"] = json!([http_record(
        "/retention-detail",
        json!({"ok":true}),
        Some(json!({"record_id":9039})),
        9001,
        at + 900200,
        at + 900300,
        None
    )]);
    send(&client, &url, token, &bounded).await;
    while doc.process_one().await.unwrap().is_some() {}
    while capture.process_evidence_one().await.unwrap() {}
    let related: i64=sqlx::query_scalar("SELECT count(*) FROM evidence_facts WHERE kind='parameter_link_candidate' AND subject->'source'->>'interface_id'=$1 AND subject->'target'->>'path'='/record_id'").bind(owner.to_string()).fetch_one(&sql).await.unwrap();
    assert!(
        related > 0,
        "value beyond retained examples still supports a relationship"
    );
    stop.cancel();
    server.await.unwrap().unwrap();
    sql.close().await;
    db.close().await;
    if let Ok(path) = std::env::var("NEXOFOLIO_KEEP_CAPTURE_FIXTURE") {
        std::fs::write(path,serde_json::to_vec(&json!({"database_url":db_url,"blob_root":blobs,"session_key":key,"instance":instance,"token":session.token.expose(),"project_id":p,"user_id":session.user.id})).unwrap()).unwrap();
        admin.close().await;
        return;
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    std::fs::remove_dir_all(blobs).unwrap();
}
