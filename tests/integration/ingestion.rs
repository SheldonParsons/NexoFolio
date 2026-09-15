use nexofolio_access::{
    ExternalIdentity, ExternalProject, PlatformAccess, ProjectSnapshot, SessionPrincipal,
};
use nexofolio_backend::{
    http,
    wiring::{Config, build_access},
};
use nexofolio_contracts::Secret;
use nexofolio_infrastructure::{Postgres, PostgresAccess, Unconfigured};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn record(batch: &Value) -> Value {
    let mut r = batch["records"][0].clone();
    r["record_id"] = json!(Uuid::new_v4());
    r
}
async fn send(client: &reqwest::Client, url: &str, token: &str, batch: &Value) -> Value {
    let r = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(batch)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let receipt: Value = r.json().await.unwrap();
    let schema: Value = serde_json::from_str(http::ingestion::RECEIPT_SCHEMA).unwrap();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&receipt)
    );
    receipt
}
#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn environment_current_structure_retries_and_downstream_handoff() {
    let db_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let db = Postgres::new(&Secret::new(&db_url), 10, Duration::from_secs(2)).unwrap();
    db.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&db_url).await.unwrap();
    let key = "12".repeat(32);
    let base = format!("http://127.0.0.1:1/fixture-{}", Uuid::new_v4());
    let store = PostgresAccess::new(db.clone(), &Secret::new(&key)).unwrap();
    let session = store
        .normal_login(&ExternalIdentity {
            instance: base.clone(),
            external_id: "actor".into(),
            account: "actor".into(),
            display_name: "actor".into(),
        })
        .await
        .unwrap();
    let actor = SessionPrincipal {
        user_id: session.user.id,
        instance: base.clone(),
    };
    let generation = store.next_sync_generation().await.unwrap();
    store
        .apply_snapshot(
            &actor,
            generation,
            ProjectSnapshot {
                projects: vec![
                    ExternalProject {
                        instance: base.clone(),
                        external_id: "p1".into(),
                        name: "P1".into(),
                        state: "doing".into(),
                    },
                    ExternalProject {
                        instance: base.clone(),
                        external_id: "p2".into(),
                        name: "P2".into(),
                        state: "doing".into(),
                    },
                ],
                visible_project_ids: vec!["p1".into()],
            },
        )
        .await
        .unwrap();
    let items = store.list_projects(&actor, 1, 100).await.unwrap().items;
    let allowed = items.iter().find(|p| p.can_access).unwrap().project_id;
    let denied = items.iter().find(|p| !p.can_access).unwrap().project_id;
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some(db_url.clone()),
        "NEXOFOLIO_ZENTAO_BASE_URL" => Some(base.clone()),
        "NEXOFOLIO_SESSION_KEY" => Some(key.clone()),
        _ => None,
    })
    .unwrap();
    let access = build_access(&config, db.clone()).unwrap();
    let stop = CancellationToken::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = http::router_with_access(
        &config,
        Arc::new(db.clone()),
        Arc::new(Unconfigured),
        stop.clone(),
        access,
    );
    let handle = tokio::spawn(http::serve(
        listener,
        app,
        stop.clone(),
        Duration::from_secs(2),
    ));
    let client = reqwest::Client::new();
    let token = session.token.expose();
    let caps = client
        .get(format!("{url}/v1/ingestion/capabilities"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(caps.status(), 200);
    let mut batch: Value = serde_json::from_str(include_str!(
        "../../contracts/ingestion/fixtures/http-batch.json"
    ))
    .unwrap();
    batch["project_id"] = json!(allowed);
    batch["source"]["instance_id"] = json!(Uuid::new_v4());
    let first = send(&client, &url, token, &batch).await;
    assert_eq!(first["results"][0]["status"], "accepted");
    let ingestion_id = first["results"][0]["ingestion_id"].as_str().unwrap();
    let raw: Value = sqlx::query_scalar("SELECT raw_record FROM ingestion_inbox WHERE id=$1")
        .bind(ingestion_id.parse::<Uuid>().unwrap())
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(raw, batch["records"][0]); // Business headers and raw values unredacted.
    let replay = send(&client, &url, token, &batch).await;
    assert_eq!(
        replay["results"][0]["ingestion_id"],
        first["results"][0]["ingestion_id"]
    );
    assert_eq!(replay["results"][0]["replayed"], true);
    batch["batch_id"] = json!(Uuid::new_v4());
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["replayed"],
        true
    );
    // Empty samples do not erase the current richer projection, within a batch or after reload.
    let mut directional = batch.clone();
    let mut empty = record(&batch);
    empty["payload"]["response"]["body"]["content"] = json!(r#"{"items":[]}"#);
    let mut empty2 = empty.clone();
    empty2["record_id"] = json!(Uuid::new_v4());
    directional["records"] = json!([empty, empty2, record(&batch)]);
    let result = send(&client, &url, token, &directional).await;
    for receipt in result["results"].as_array().unwrap() {
        assert_eq!(receipt["status"], "ignored");
        assert_eq!(receipt["ingestion_id"], first["results"][0]["ingestion_id"]);
    }
    directional["records"] = json!([record(&batch)]);
    assert_eq!(
        send(&client, &url, token, &directional).await["results"][0]["status"],
        "ignored"
    );
    // Simulate an existing v1 head. Upgrade lazily from its raw current observation, not history.
    sqlx::query("UPDATE ingestion_heads SET algorithm_version='http-structure-1',structural_projection=NULL,structural_hash=NULL WHERE ingestion_id=$1")
        .bind(ingestion_id.parse::<Uuid>().unwrap()).execute(&sql).await.unwrap();
    directional["records"] = json!([record(&batch)]);
    assert_eq!(
        send(&client, &url, token, &directional).await["results"][0]["status"],
        "ignored"
    );
    let other_session = store
        .normal_login(&ExternalIdentity {
            instance: base.clone(),
            external_id: "other-user".into(),
            account: "other-user".into(),
            display_name: "Other".into(),
        })
        .await
        .unwrap();
    let other = SessionPrincipal {
        user_id: other_session.user.id,
        instance: base.clone(),
    };
    store
        .apply_snapshot(
            &other,
            store.next_sync_generation().await.unwrap(),
            ProjectSnapshot {
                projects: vec![],
                visible_project_ids: vec!["p2".into()],
            },
        )
        .await
        .unwrap();
    let other_env = store
        .create_environment(&other, denied, "开发环境")
        .await
        .unwrap();
    assert_ne!(
        serde_json::to_value(other_env.id).unwrap(),
        first["environment"]["id"]
    );
    let mut wrong_project_env = batch.clone();
    wrong_project_env["environment"] = json!({"id":other_env.id});
    wrong_project_env["records"][0] = record(&batch);
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&wrong_project_env)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    // v1 fields are accepted only for immutable old queues, never a dedup partition.
    let mut legacy = batch.clone();
    legacy["schema_version"] = json!("1");
    legacy["service_key"] = json!("old-service-a");
    legacy["records"][0] = record(&batch);
    let legacy_response = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(&legacy)
        .send()
        .await
        .unwrap();
    assert_eq!(legacy_response.status(), 200);
    let legacy_response: Value = legacy_response.json().await.unwrap();
    assert_eq!(legacy_response["schema_version"], "1");
    assert_eq!(legacy_response["results"][0]["status"], "ignored");
    let response = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(&legacy)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(response["results"][0]["replayed"], true);
    legacy["service_key"] = json!("old-service-b");
    legacy["records"][0] = record(&batch);
    let response = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(&legacy)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(response["results"][0]["status"], "ignored");
    let mut invalid_v2 = batch.clone();
    invalid_v2["service_key"] = json!("default");
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&invalid_v2)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let original = batch.clone();
    let env_id = first["environment"]["id"].as_str().unwrap().to_owned();
    let env_url = format!("{url}/v1/projects/{allowed}/environments");
    let envs = client
        .get(&env_url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(envs.status(), 200);
    let envs: Value = envs.json().await.unwrap();
    assert_eq!(envs["total"], 1);
    assert_eq!(envs["items"][0]["name"], "开发环境");
    let renamed = client
        .patch(format!("{env_url}/{env_id}"))
        .bearer_auth(token)
        .json(&json!({"name":"测试环境"}))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), 200);
    let rename: Value = renamed.json().await.unwrap();
    assert_eq!(rename["id"], first["environment"]["id"]);
    // Old queued name retries and ID-based new uploads resolve to the same stable identity.
    let old_name = send(&client, &url, token, &original).await;
    assert_eq!(old_name["results"][0]["replayed"], true);
    assert_eq!(old_name["environment"]["name"], "测试环境");
    let mut by_id = original.clone();
    by_id["environment"] = json!({"id":env_id});
    assert_eq!(
        send(&client, &url, token, &by_id).await["results"][0]["replayed"],
        true
    );
    let created = client
        .post(&env_url)
        .bearer_auth(token)
        .json(&json!({"name":"预发布环境"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let created: Value = created.json().await.unwrap();
    let same = client
        .post(&env_url)
        .bearer_auth(token)
        .json(&json!({"name":"预发布环境"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(same["id"], created["id"]);
    assert_eq!(
        client
            .patch(format!("{env_url}/{}", created["id"].as_str().unwrap()))
            .bearer_auth(token)
            .json(&json!({"name":"开发环境"}))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        client
            .post(&env_url)
            .bearer_auth(token)
            .json(&json!({"name":" 生产 "}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/projects/{denied}/environments"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );

    batch["environment"] = json!({"name":"生产环境"});
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["reason_code"],
        "IDEMPOTENCY_CONFLICT"
    );
    batch["records"][0] = record(&original);
    let production = send(&client, &url, token, &batch).await;
    assert_eq!(production["results"][0]["status"], "accepted");
    batch = original.clone();
    batch["records"][0] = record(&original);
    batch["records"][0]["payload"]["request"]["url"] =
        json!("https://another.example.test/orders?page=99");
    batch["records"][0]["payload"]["response"]["body"]["content"] =
        json!("{\"items\":[{\"id\":999,\"name\":\"changed value\"}]}");
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["status"],
        "ignored"
    );
    // B then A: all-time history is NOT a dedup set.
    batch["records"][0] = record(&original);
    batch["records"][0]["payload"]["response"]["body"]["content"] =
        json!("{\"items\":[{\"id\":1,\"new_field\":true}]}");
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["status"],
        "accepted"
    );
    batch["records"][0] = record(&original);
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["status"],
        "accepted"
    );
    // Incomplete samples are forwarded, never declared structurally equivalent.
    batch["records"][0] = record(&original);
    batch["records"][0]["payload"]["response"]["state"] = json!("truncated");
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["status"],
        "accepted"
    );
    batch["records"][0] = record(&original);
    assert_eq!(
        send(&client, &url, token, &batch).await["results"][0]["status"],
        "accepted"
    );
    // Valid item survives an unsupported sibling; invalid items get explicit terminal receipts.
    let valid = record(&original);
    let mut unknown = record(&original);
    unknown["kind"] = json!("image");
    batch["records"] = json!([valid,unknown,{"broken":true}]);
    let mixed = send(&client, &url, token, &batch).await;
    assert_eq!(mixed["results"].as_array().unwrap().len(), 3);
    assert_eq!(mixed["results"][0]["status"], "ignored");
    assert_eq!(mixed["results"][1]["reason_code"], "UNSUPPORTED_KIND");
    assert_eq!(mixed["results"][2]["record_id"], Value::Null);
    // Concurrent identical new structure produces one downstream entry and one replay.
    batch["records"] = json!([record(&original)]);
    batch["environment"] = json!({"name":"concurrent"});
    let (a, b) = tokio::join!(
        send(&client, &url, token, &batch),
        send(&client, &url, token, &batch)
    );
    assert_eq!(
        a["results"][0]["ingestion_id"],
        b["results"][0]["ingestion_id"]
    );
    assert!(a["results"][0]["replayed"] == true || b["results"][0]["replayed"] == true);
    // UUID changes but same current shape admits no extra payloads; record-ID receipts persist.
    let mut large = original.clone();
    large["environment"] = json!({"name":"throughput"});
    large["records"] = json!((0..50).map(|_| record(&original)).collect::<Vec<_>>());
    let start = std::time::Instant::now();
    let receipts = send(&client, &url, token, &large).await;
    assert_eq!(
        receipts["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["status"] == "accepted")
            .count(),
        1
    );
    assert_eq!(
        receipts["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["status"] == "ignored")
            .count(),
        49
    );
    println!(
        "50-item synthetic batch elapsed_ms={}",
        start.elapsed().as_millis()
    );
    // Access checks before comparisons, also with a known replay.
    let mut forbidden = original.clone();
    forbidden["project_id"] = json!(denied);
    let status = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .json(&forbidden)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 403);
    let mut no_env = original.clone();
    no_env.as_object_mut().unwrap().remove("environment");
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&no_env)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .json(&original)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .header("Content-Encoding", "gzip")
            .json(&original)
            .send()
            .await
            .unwrap()
            .status(),
        415
    );
    let mut unsupported = original.clone();
    unsupported["records"][0]["kind"] = json!("image");
    let empty_result = send(&client, &url, token, &unsupported).await;
    assert_eq!(empty_result["environment"], Value::Null);
    let mut unknown_env = original.clone();
    unknown_env["environment"] = json!({"id":Uuid::new_v4()});
    unknown_env["records"][0] = record(&original);
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&unknown_env)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let mut too_many = original.clone();
    too_many["records"] = json!((0..51).map(|_| record(&original)).collect::<Vec<_>>());
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&too_many)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let mut oversized = original.clone();
    oversized["records"][0] = record(&original);
    oversized["records"][0]["payload"]["response"]["body"]["content"] = json!("中".repeat(1450000));
    assert_eq!(
        send(&client, &url, token, &oversized).await["results"][0]["reason_code"],
        "RECORD_TOO_LARGE"
    );
    let huge = client
        .post(format!("{url}/v1/ingestion/batches"))
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .body(" ".repeat(8 * 1024 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(huge.status(), 413);
    // A database failure after preparation must roll back every valid record in the batch.
    sqlx::query("ALTER TABLE ingestion_inbox ADD CONSTRAINT fixture_reject_env CHECK (identity_key <> 'GET /force-db-failure')").execute(&sql).await.unwrap();
    let mut rollback = original.clone();
    rollback["environment"] = json!({"name":"rollback-only"});
    let a = record(&original);
    let mut bad = record(&original);
    bad["payload"]["request"]["url"] = json!("https://api.example.test/force-db-failure");
    rollback["records"] = json!([a, bad]);
    assert_eq!(
        client
            .post(format!("{url}/v1/ingestion/batches"))
            .bearer_auth(token)
            .json(&rollback)
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    let project_uuid: Uuid = allowed.to_string().parse().unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM environments WHERE project_id=$1 AND name='rollback-only'",
    )
    .bind(project_uuid)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(count, 0);
    sqlx::query("ALTER TABLE ingestion_inbox DROP CONSTRAINT fixture_reject_env")
        .execute(&sql)
        .await
        .unwrap();
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ingestion_inbox WHERE project_id=$1 AND status='pending'",
    )
    .bind(project_uuid)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert!(pending >= 7);
    let processed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ingestion_inbox WHERE project_id=$1 AND status<>'pending'",
    )
    .bind(project_uuid)
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(processed, 0);
    directional["environment"] = json!({"name":"empty-first"});
    let mut empty = record(&batch);
    empty["payload"]["response"]["body"]["content"] = json!(r#"{"items":[]}"#);
    directional["records"] = json!([empty, record(&batch)]);
    let result = send(&client, &url, token, &directional).await;
    assert_eq!(result["results"][0]["status"], "accepted");
    assert_eq!(result["results"][1]["status"], "accepted");
    let mut changed = record(&batch);
    changed["payload"]["response"]["body"]["content"] = json!(r#"{"items":[],"added":true}"#);
    directional["records"] = json!([changed]);
    assert_eq!(
        send(&client, &url, token, &directional).await["results"][0]["status"],
        "accepted"
    );
    // First numeric path observation establishes a template; values do not affect retries.
    let mut paths = original.clone();
    paths["environment"] = json!({"name":"path-normalization"});
    let mut a = record(&original);
    a["payload"]["request"]["url"] = json!("https://api.example.test/path-users/123");
    let mut b = record(&original);
    b["payload"]["request"]["url"] = json!("https://api.example.test/path-users/456");
    paths["records"] = json!([a, b]);
    let answer = send(&client, &url, token, &paths).await;
    assert_eq!(answer["results"][0]["status"], "accepted");
    assert_eq!(answer["results"][1]["status"], "ignored");
    let id: Uuid = answer["results"][0]["ingestion_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let identity: Value =
        sqlx::query_scalar("SELECT path_identity FROM ingestion_inbox WHERE id=$1")
            .bind(id)
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(identity["template"], "/path-users/{param1}");
    let raw: Value = sqlx::query_scalar("SELECT raw_record FROM ingestion_inbox WHERE id=$1")
        .bind(id)
        .fetch_one(&sql)
        .await
        .unwrap();
    assert_eq!(raw, paths["records"][0]);
    assert_eq!(
        send(&client, &url, token, &paths).await["results"][0]["replayed"],
        true
    );
    let mut changed = record(&original);
    changed["payload"]["request"]["url"] = json!("https://api.example.test/path-users/789");
    changed["payload"]["response"]["body"]["content"] = json!(r#"{"items":[],"new_field":true}"#);
    paths["records"] = json!([changed]);
    assert_eq!(
        send(&client, &url, token, &paths).await["results"][0]["status"],
        "accepted"
    );
    use nexofolio_intake::ProjectPathPolicies;
    let policies = nexofolio_infrastructure::PostgresPathPolicies::new(db.clone());
    policies
        .set(
            allowed,
            &nexofolio_contracts::PathPolicy {
                literal_prefixes: vec!["/path-users".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut literal = record(&original);
    literal["payload"]["request"]["url"] = json!("https://api.example.test/path-users/999");
    paths["records"] = json!([literal]);
    assert_eq!(
        send(&client, &url, token, &paths).await["results"][0]["status"],
        "accepted"
    );
    policies.set(allowed, &Default::default()).await.unwrap();
    stop.cancel();
    handle.await.unwrap().unwrap();
    // Fresh adapters / server retain receipts and heads after restart.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let app = http::router_with_access(
        &config,
        Arc::new(db.clone()),
        Arc::new(Unconfigured),
        stop.clone(),
        build_access(&config, db.clone()).unwrap(),
    );
    let handle = tokio::spawn(http::serve(
        listener,
        app,
        stop.clone(),
        Duration::from_secs(2),
    ));
    assert_eq!(
        send(&client, &url, token, &original).await["results"][0]["replayed"],
        true
    );
    stop.cancel();
    handle.await.unwrap().unwrap();
    sql.close().await;
    db.close().await;
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn migration_collapses_service_heads_without_losing_observations_or_receipts() {
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "CREATE SCHEMA scope_migration_fixture; SET LOCAL search_path TO scope_migration_fixture",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    for script in [
        include_str!("../../migrations/202609140001_access.sql"),
        include_str!("../../migrations/202609140002_environments.sql"),
        include_str!("../../migrations/202609140003_ingestion.sql"),
    ] {
        sqlx::raw_sql(script).execute(&mut *tx).await.unwrap();
    }
    let user = Uuid::new_v4();
    let project = Uuid::new_v4();
    let env = Uuid::new_v4();
    let old = Uuid::new_v4();
    let latest = Uuid::new_v4();
    let record = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,instance,external_id,account,display_name) VALUES($1,'fixture','u','u','u')").bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,'fixture','p','p','doing',1)").bind(project).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO environments(id,project_id,name) VALUES($1,$2,'开发环境')")
        .bind(env)
        .bind(project)
        .execute(&mut *tx)
        .await
        .unwrap();
    for (id, service, time) in [
        (old, "a", "2026-01-01T00:00:00Z"),
        (latest, "b", "2026-02-01T00:00:00Z"),
    ] {
        sqlx::query("INSERT INTO ingestion_inbox(id,project_id,actor_id,producer_id,source_type,record_id,batch_id,environment_id,service_key,identity_key,structural_hash,raw_record,received_at) VALUES($1,$2,$3,$4,'fixture',$5,$6,$7,$8,'GET /orders',$9,'{}'::jsonb,$10::text::timestamptz)").bind(id).bind(project).bind(user).bind(Uuid::new_v4()).bind(record).bind(Uuid::new_v4()).bind(env).bind(service).bind(vec![1u8]).bind(time).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO ingestion_heads(project_id,environment_id,service_key,identity_key,algorithm_version,structural_hash,ingestion_id) VALUES($1,$2,$3,'GET /orders','http-structure-1',$4,$5)").bind(project).bind(env).bind(service).bind(vec![1u8]).bind(id).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("INSERT INTO ingestion_receipts(actor_id,producer_id,record_id,content_hash,status,reason_code,ingestion_id) VALUES($1,$2,$3,$4,'accepted','FORWARDED',$5)").bind(user).bind(Uuid::new_v4()).bind(record).bind(vec![7u8]).bind(old).execute(&mut *tx).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/202609140004_project_environment_scope.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let head: Uuid = sqlx::query_scalar("SELECT ingestion_id FROM ingestion_heads")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(head, latest);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM ingestion_inbox")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let hash: Vec<u8> = sqlx::query_scalar("SELECT content_hash FROM ingestion_receipts")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(hash, vec![7u8]);
    // Roll back the entire isolated schema and fixtures; never touch the public application data.
    tx.rollback().await.unwrap();
    pool.close().await;
}
