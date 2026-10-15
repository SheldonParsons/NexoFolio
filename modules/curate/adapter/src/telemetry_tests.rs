use super::*;
use nexofolio_curate_contracts::telemetry::ModelTelemetry;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn actual_429_and_503_calls_are_classified_without_changing_budget_or_issuing_on_read() {
    for status in [429, 503] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            assert!(stream.read(&mut buffer).await.unwrap() > 0);
            let response = format!(
                "HTTP/1.1 {status} Error\r\nContent-Length: 0\r\nRetry-After: 45\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            // Snapshots and shared cooldown must not initiate another HTTP call.
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept())
                    .await
                    .is_err()
            );
        });
        let client = LlmClient::new(base, "fixture".into(), "fixture".into()).with_concurrency(4);
        assert_eq!(client.model_snapshot().unwrap().request.mean_ms, None);
        assert!(
            matches!(client.complete("synthetic").await,Err(LlmError::Status{status:s,..}) if s==status)
        );
        let before = client.model_snapshot().unwrap();
        assert_eq!(before.http_in_flight, 0);
        assert_eq!(before.permits_in_use, 0);
        assert_eq!(before.current_limit, 1);
        assert_eq!(before.configured_limit, 4);
        assert_eq!(before.request.samples, 1);
        assert!(before.cooldown_remaining_ms > 44000);
        assert_eq!(
            before.failures[if status == 429 {
                "model_rate_limited"
            } else {
                "model_unavailable"
            }],
            1
        );
        for _ in 0..100 {
            let s = client.model_snapshot().unwrap();
            assert_eq!(s.current_limit, before.current_limit);
            assert_eq!(s.completed, before.completed);
        }
        assert!(matches!(
            client.complete("synthetic").await,
            Err(LlmError::Cooldown { .. })
        ));
        let after = client.model_snapshot().unwrap();
        assert_eq!(after.completed, 2);
        assert_eq!(after.request.samples, 1);
        assert_eq!(after.failures["model_cooldown"], 1);
        server.await.unwrap();
    }
}
#[tokio::test]
async fn dropped_real_http_future_releases_only_its_observation_and_permit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = std::sync::Arc::new(
        LlmClient::new(
            format!("http://{}", listener.local_addr().unwrap()),
            "fixture".into(),
            "fixture".into(),
        )
        .with_concurrency(1),
    );
    let c = client.clone();
    let running = tokio::spawn(async move { c.complete("synthetic").await });
    let (stream, _) = listener.accept().await.unwrap();
    let c = client.clone();
    let waiting = tokio::spawn(async move { c.complete("synthetic").await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let s = client.model_snapshot().unwrap();
    assert_eq!(s.http_in_flight, 1);
    assert_eq!(s.waiting_for_permit, 1);
    waiting.abort();
    let _ = waiting.await;
    running.abort();
    let _ = running.await;
    drop(stream);
    let s = client.model_snapshot().unwrap();
    assert_eq!(s.http_in_flight, 0);
    assert_eq!(s.waiting_for_permit, 0);
    assert_eq!(s.permits_in_use, 0);
    assert_eq!(s.cancelled, 2);
    assert_eq!(s.current_limit, 1);
}
