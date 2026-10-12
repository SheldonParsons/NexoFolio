//! Real TCP/JSON-RPC boundary with test-only identity and synthetic knowledge.
use super::*;
use async_trait::async_trait;
use nexofolio_access_contracts::{McpPrincipal, McpTokenVerifier};
use nexofolio_common::{DatabaseProbe, Secret, TokenId, UserId};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
struct Probe;
#[async_trait]
impl DatabaseProbe for Probe {
    async fn check(&self) -> nexofolio_common::Result<()> {
        Ok(())
    }
}
struct Verifier;
#[async_trait]
impl McpTokenVerifier for Verifier {
    async fn verify(&self, s: &Secret) -> nexofolio_common::Result<McpPrincipal> {
        if s.expose() != "synthetic-exploration-only" {
            return Err(nexofolio_common::Error::Unauthenticated);
        }
        Ok(McpPrincipal {
            user_id: UserId::new(),
            token_id: TokenId::new(),
        })
    }
}
#[tokio::test]
async fn real_transport_exposes_eleven_tools_and_followable_directory_reads() {
    let f = setup().await;
    let config = crate::wiring::Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://unused@127.0.0.1/unused".into())
    })
    .unwrap();
    let stop = CancellationToken::new();
    let app = crate::http::router_with_navigation(
        &config,
        Arc::new(Probe),
        Arc::new(Verifier),
        stop.clone(),
        None,
        Some(f.nav),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(crate::http::serve(
        listener,
        app,
        stop.clone(),
        Duration::from_secs(1),
    ));
    let client = reqwest::Client::new();
    let invoke = |method: &str, params: Value| {
        client
            .post(&url)
            .bearer_auth("synthetic-exploration-only")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
    };
    let init:Value=invoke("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic-exploration","version":"1"}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(
        init["result"]["instructions"].as_str().unwrap(),
        include_str!("../instructions.md"),
        "the real initialize response must carry the complete service guidance"
    );
    let listed: Value = invoke("tools/list", json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 11);
    let response:Value=invoke("tools/call",json!({"name":"browse_directory","arguments":{"project_id":f.project,"environment_id":f.env,"limit":1}})).send().await.unwrap().json().await.unwrap();
    let read = &response["result"]["structuredContent"];
    assert_eq!(read["items"][0]["name"], "订单");
    let next = &read["items"][0]["open"];
    let response: Value = invoke(
        "tools/call",
        json!({"name":next["tool"],"arguments":next["arguments"]}),
    )
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        response["result"]["structuredContent"]["items"][0]["name"],
        "创建订单"
    );
    let error:Value=invoke("tools/call",json!({"name":"read_source_evidence","arguments":{"project_id":f.project,"endpoint_id":f.target,"max_chars":20000}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(error["result"]["isError"], true);
    assert_eq!(
        error["result"]["structuredContent"]["error"],
        "INVALID_BOUNDS"
    );
    assert_eq!(
        error["result"]["structuredContent"]["details"]["parameter"],
        "max_chars"
    );
    assert_eq!(
        error["result"]["structuredContent"]["details"]["maximum"],
        16384
    );
    stop.cancel();
    assert!(server.await.unwrap().is_ok());
}
