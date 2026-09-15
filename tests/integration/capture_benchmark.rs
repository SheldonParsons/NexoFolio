use nexofolio_backend::{
    http,
    wiring::{Config, build_access},
};
use nexofolio_contracts::Secret;
use nexofolio_infrastructure::{Postgres, Unconfigured};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
pub async fn compare(
    database_url: &str,
    instance: &str,
    key: &str,
    token: &str,
    blob_root: &std::path::Path,
    template: &Value,
) {
    if std::env::var("NEXOFOLIO_CAPTURE_BENCHMARK").is_err() {
        return;
    }
    let mut results = vec![];
    for enabled in [false, true] {
        let config = Config::from_lookup(|name| match name {
            "DATABASE_URL" => Some(database_url.into()),
            "NEXOFOLIO_ZENTAO_BASE_URL" => Some(instance.into()),
            "NEXOFOLIO_SESSION_KEY" => Some(key.into()),
            "NEXOFOLIO_CAPTURE_ENABLED" => Some(enabled.to_string()),
            "NEXOFOLIO_BLOB_ROOT" => Some(blob_root.to_string_lossy().into()),
            _ => None,
        })
        .unwrap();
        let database =
            Postgres::new(&Secret::new(database_url), 5, Duration::from_secs(2)).unwrap();
        let shutdown = CancellationToken::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v1/ingestion/batches",
            listener.local_addr().unwrap()
        );
        let router = http::router_with_access(
            &config,
            Arc::new(database.clone()),
            Arc::new(Unconfigured),
            shutdown.clone(),
            build_access(&config, database.clone()).unwrap(),
        );
        let task = tokio::spawn(http::serve(
            listener,
            router,
            shutdown.clone(),
            Duration::from_secs(2),
        ));
        let client = reqwest::Client::new();
        let mut input = template.clone();
        input["schema_version"] = json!(if enabled { "3" } else { "2" });
        input["source"]["instance_id"] = json!(Uuid::new_v4());
        let mut record = input["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "http_exchange")
            .unwrap()
            .clone();
        record.as_object_mut().unwrap().remove("context");
        let mut times = vec![];
        let mut body_bytes = 0;
        for sample in 0..25 {
            input["batch_id"] = json!(Uuid::new_v4());
            input["records"] = json!(
                (0..20)
                    .map(|_| {
                        let mut r = record.clone();
                        r["record_id"] = json!(Uuid::new_v4());
                        r
                    })
                    .collect::<Vec<_>>()
            );
            body_bytes = serde_json::to_vec(&input).unwrap().len();
            let start = Instant::now();
            let response = client
                .post(&url)
                .bearer_auth(token)
                .json(&input)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let body: Value = response.json().await.unwrap();
            assert!(
                body["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["status"] == if enabled { "accepted" } else { "ignored" })
            );
            if sample >= 5 {
                times.push(start.elapsed().as_secs_f64() * 1000.0)
            }
        }
        times.sort_by(f64::total_cmp);
        results.push(json!({"mode":if enabled{"durable_evidence"}else{"structure_only"},"batches":times.len(),"records_per_batch":20,"request_bytes":body_bytes,"concurrency":1,"p50_ms":times[times.len()/2],"p95_ms":times[times.len()*95/100],"mean_ms":times.iter().sum::<f64>()/times.len() as f64,"records_per_second":20000.0/(times.iter().sum::<f64>()/times.len() as f64)}));
        shutdown.cancel();
        task.await.unwrap().unwrap();
        database.close().await;
    }
    let result = json!({"source":"synthetic local HTTP and isolated PostgreSQL; no evidence worker or LLM in timing window","results":results});
    let path = std::env::var("NEXOFOLIO_CAPTURE_BENCHMARK").unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}
