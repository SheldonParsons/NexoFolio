//! Explicit local acceptance replay. Input is a private export, never a checked-in fixture.
use nexofolio_access::{
    ExternalIdentity, ExternalProject, PlatformAccess, ProjectSnapshot, SessionPrincipal,
};
use nexofolio_application::ProcessingService;
use nexofolio_backend::{
    http,
    wiring::{Config, build_access},
};
use nexofolio_contracts::Secret;
use nexofolio_infrastructure::{
    Postgres, PostgresAccess, PostgresCatalogPreviews, PostgresDocuments, Unconfigured,
};
use nexofolio_knowledge::CatalogPreviewStore;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[tokio::test]
#[ignore = "requires private NEXOFOLIO_REPLAY_FILE and isolated TEST_DATABASE_URL"]
async fn real_observations_replay_through_http_and_worker() {
    let original = std::env::var("TEST_DATABASE_URL").expect("isolated database required");
    assert!(
        !original.contains(":15432/"),
        "production database is not a replay target"
    );
    let input: Vec<Value> = serde_json::from_slice(
        &std::fs::read(std::env::var("NEXOFOLIO_REPLAY_FILE").unwrap()).unwrap(),
    )
    .unwrap();
    assert!(!input.is_empty());
    let admin = sqlx::PgPool::connect(&original).await.unwrap();
    let schema = format!("replay_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let sep = if original.contains('?') { "&" } else { "?" };
    let url = format!("{original}{sep}options=-csearch_path%3D{schema}");
    let db = Postgres::new(&Secret::new(&url), 10, Duration::from_secs(2)).unwrap();
    db.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&url).await.unwrap();
    let instance = "http://127.0.0.1:1/replay";
    let key = "f1".repeat(32);
    let store = Arc::new(PostgresAccess::new(db.clone(), &Secret::new(&key)).unwrap());
    let mut sessions = HashMap::new();
    let mut project = None;
    for row in &input {
        let actor = row["actor_id"].as_str().unwrap();
        if sessions.contains_key(actor) {
            continue;
        }
        let session = store
            .normal_login(&ExternalIdentity {
                instance: instance.into(),
                external_id: actor.into(),
                account: actor.into(),
                display_name: "Replay user".into(),
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
                    projects: vec![ExternalProject {
                        instance: instance.into(),
                        external_id: "replay-project".into(),
                        name: "Isolated observation replay".into(),
                        state: "doing".into(),
                    }],
                    visible_project_ids: vec!["replay-project".into()],
                },
            )
            .await
            .unwrap();
        project = Some(store.list_projects(&principal, 1, 10).await.unwrap().items[0].project_id);
        sessions.insert(actor.to_owned(), session);
    }
    let project = project.unwrap();
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some(url.clone()),
        "NEXOFOLIO_ZENTAO_BASE_URL" => Some(instance.into()),
        "NEXOFOLIO_SESSION_KEY" => Some(key.clone()),
        _ => None,
    })
    .unwrap();
    let stop = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = http::router_with_access(
        &config,
        Arc::new(db.clone()),
        Arc::new(Unconfigured),
        stop.clone(),
        build_access(&config, db.clone()).unwrap(),
    );
    let server = tokio::spawn(http::serve(
        listener,
        router,
        stop.clone(),
        Duration::from_secs(2),
    ));
    let client = reqwest::Client::new();
    let mut receipts = Vec::new();
    let mut accepted = 0;
    let mut ignored = 0;
    let mut inconclusive = 0;
    for row in &input {
        let batch = json!({"schema_version":"2","batch_id":Uuid::new_v4(),"project_id":project,"environment":{"name":row["environment_name"]},"source":{"type":row["source_type"],"instance_id":row["producer_id"]},"records":[row["raw_record"]]});
        let prepared: nexofolio_intake::IngestionBatch =
            serde_json::from_value(batch.clone()).unwrap();
        if nexofolio_intake::prepare_http(&prepared, 0)
            .unwrap()
            .structural_hash
            .is_none()
        {
            inconclusive += 1;
        }
        let response = client
            .post(format!("{base}/v1/ingestion/batches"))
            .bearer_auth(sessions[row["actor_id"].as_str().unwrap()].token.expose())
            .json(&batch)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let response: Value = response.json().await.unwrap();
        let result = response["results"][0].clone();
        match result["status"].as_str().unwrap() {
            "accepted" => {
                accepted += 1;
                let id: Uuid = result["ingestion_id"].as_str().unwrap().parse().unwrap();
                let raw: Value =
                    sqlx::query_scalar("SELECT raw_record FROM ingestion_inbox WHERE id=$1")
                        .bind(id)
                        .fetch_one(&sql)
                        .await
                        .unwrap();
                assert_eq!(raw, row["raw_record"]);
            }
            "ignored" => ignored += 1,
            _ => panic!("replay unexpectedly rejected a captured record"),
        };
        receipts.push(json!({"source_observation":row["id"],"status":result["status"],"ingestion_id":result["ingestion_id"],"reason_code":result["reason_code"]}));
    }
    let processor = Arc::new(PostgresDocuments::new(db.clone()));
    let service = ProcessingService::new(processor);
    let mut outcomes = HashMap::<String, usize>::new();
    while let Some(result) = service.process_one().await.unwrap() {
        *outcomes.entry(result.outcome).or_default() += 1;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM interface_documents")
        .fetch_one(&sql)
        .await
        .unwrap();
    let groups: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM interface_documents WHERE path LIKE '%/table-config-info/find/%'",
    )
    .fetch_one(&sql)
    .await
    .unwrap();
    assert_eq!(groups, 1);
    let path: String = sqlx::query_scalar(
        "SELECT path FROM interface_documents WHERE path LIKE '%/table-config-info/find/%'",
    )
    .fetch_one(&sql)
    .await
    .unwrap();
    assert!(path.ends_with("/{param1}"));
    let completed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ingestion_inbox WHERE status='completed'")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(completed, accepted);
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ingestion_inbox WHERE status<>'completed'")
            .fetch_one(&sql)
            .await
            .unwrap();
    assert_eq!(pending, 0);
    let differences: i64 =
        sqlx::query_scalar("SELECT count(*) FROM interface_observed_differences")
            .fetch_one(&sql)
            .await
            .unwrap();
    let task = PostgresCatalogPreviews::new(db.clone())
        .create(project)
        .await
        .unwrap();
    let report = json!({"input_observations":input.len(),"accepted":accepted,"ignored":ignored,"inconclusive_at_admission":inconclusive,"documents":count,"table_config_documents":groups,"completed":completed,"outcomes":outcomes,"pending_differences":differences,"raw_accepted_records_equal":true,"task_id":task.task_id,"project_id":project,"schema":schema,"receipts":receipts});
    std::fs::write(
        std::env::var("NEXOFOLIO_REPLAY_REPORT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    // Private connection metadata enables a separately authorized model pass on this isolated data.
    std::fs::write(
        std::env::var("NEXOFOLIO_REPLAY_CONNECTION").unwrap(),
        serde_json::to_vec(&json!({"url":url,"task_id":task.task_id,"project_id":project}))
            .unwrap(),
    )
    .unwrap();
    println!(
        "Replay: {} observations, {accepted} accepted, {ignored} ignored, {count} documents",
        input.len()
    );
    stop.cancel();
    server.await.unwrap().unwrap();
    sql.close().await;
    db.close().await;
    admin.close().await;
}
