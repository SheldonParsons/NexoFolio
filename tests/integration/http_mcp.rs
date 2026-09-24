use async_trait::async_trait;
use nexofolio_access_adapter::Unconfigured;
use nexofolio_access_contracts::{McpPrincipal, McpTokenVerifier};
use nexofolio_backend::{
    http,
    wiring::{Config, run_worker},
};
use nexofolio_common::DatabaseProbe;
use nexofolio_common::{Error, Result, Secret, TokenId, UserId};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

struct Probe(AtomicBool);
#[async_trait]
impl DatabaseProbe for Probe {
    async fn check(&self) -> Result<()> {
        if self.0.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::Unavailable {
                component: "database",
            })
        }
    }
}

// This credential and identity exist only in this test target; no runtime override.
struct TestVerifier;
#[async_trait]
impl McpTokenVerifier for TestVerifier {
    async fn verify(&self, token: &Secret) -> Result<McpPrincipal> {
        if token.expose() != "fixture-only-credential" {
            return Err(Error::Unauthenticated);
        }
        Ok(McpPrincipal {
            user_id: UserId::new(),
            token_id: TokenId::new(),
        })
    }
}

fn config() -> Config {
    Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some("postgres://unused@127.0.0.1/unused".into()),
        "NEXOFOLIO_DB_TIMEOUT_MS" => Some("50".into()),
        _ => None,
    })
    .unwrap()
}

async fn start(
    probe: Arc<dyn DatabaseProbe>,
    verifier: Arc<dyn McpTokenVerifier>,
) -> (
    String,
    CancellationToken,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let config = config();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let router = http::router(&config, probe, verifier, stop.clone());
    let handle = tokio::spawn(http::serve(
        listener,
        router,
        stop.clone(),
        Duration::from_secs(1),
    ));
    (url, stop, handle)
}

#[tokio::test]
async fn health_changes_independently_of_liveness_and_default_mcp_is_closed() {
    let probe = Arc::new(Probe(AtomicBool::new(false)));
    let (url, stop, handle) = start(probe.clone(), Arc::new(Unconfigured)).await;
    let client = reqwest::Client::new();
    let live = client
        .get(format!("{url}/health/live"))
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), 200);
    assert!(live.headers().contains_key("x-request-id"));
    assert_eq!(
        client
            .get(format!("{url}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    probe.0.store(true, Ordering::SeqCst);
    assert_eq!(
        client
            .get(format!("{url}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for method in [
        reqwest::Method::GET,
        reqwest::Method::POST,
        reqwest::Method::DELETE,
    ] {
        let response = client
            .request(method, format!("{url}/mcp"))
            .bearer_auth("any-value")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert!(response.headers().contains_key("www-authenticate"));
        assert!(!response.text().await.unwrap().contains("any-value"));
    }
    for path in [
        "/v1/auth/login",
        "/v1/projects",
        "/v1/ingestion/batches",
        "/v1/captures/batch",
        "/v1/projects/example/maintenance-runs",
    ] {
        assert_eq!(
            client
                .post(format!("{url}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    stop.cancel();
    assert!(handle.await.unwrap().is_ok());
}

#[tokio::test]
async fn actual_http_mcp_initialize_and_empty_tool_list_with_test_only_identity() {
    let (url, stop, handle) = start(
        Arc::new(Probe(AtomicBool::new(true))),
        Arc::new(TestVerifier),
    )
    .await;
    let client = reqwest::Client::new();
    for auth in [
        None,
        Some("Bearer wrong"),
        Some("Basic fixture-only-credential"),
        Some("Bearer "),
    ] {
        let request = client.post(format!("{url}/mcp"));
        let request = if let Some(auth) = auth {
            request.header("authorization", auth)
        } else {
            request
        };
        assert_eq!(request.send().await.unwrap().status(), 401);
    }
    let invoke = |body: Value| {
        client
            .post(format!("{url}/mcp"))
            .bearer_auth("fixture-only-credential")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .json(&body)
    };
    let response = invoke(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"foundation-test","version":"1"}
    }})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let result: Value = response.json().await.unwrap();
    assert_eq!(
        result["result"]["serverInfo"]["name"],
        "nexofolio-foundation"
    );
    let response = invoke(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["result"]["tools"], json!([]));
    // Valid authentication cannot bypass the SDK's Host/Origin checks.
    let response = invoke(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
        .header("origin", "https://not-allowed.invalid")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error());
    stop.cancel();
    assert!(handle.await.unwrap().is_ok());
}

struct HangingProbe;
#[async_trait]
impl DatabaseProbe for HangingProbe {
    async fn check(&self) -> Result<()> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn readiness_probe_is_bounded_and_worker_cancels() {
    let (url, stop, handle) = start(Arc::new(HangingProbe), Arc::new(Unconfigured)).await;
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        reqwest::get(format!("{url}/health/ready")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 503);
    stop.cancel();
    assert!(handle.await.unwrap().is_ok());
    let stop = CancellationToken::new();
    let worker = tokio::spawn(run_worker(stop.clone()));
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
}
