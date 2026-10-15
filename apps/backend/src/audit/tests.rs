use super::*;
use async_trait::async_trait;
use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use nexofolio_access_contracts::{SessionPrincipal, Sessions, UserProfile};
use nexofolio_common::{Secret, UserId};
use tower::ServiceExt;

#[derive(Default)]
struct Store {
    events: Mutex<Vec<Event>>,
    reads: AtomicU64,
}
#[async_trait]
impl AuditStore for Store {
    async fn write(&self, e: &[Event], _: &Limits) -> Result<()> {
        self.events.lock().unwrap().extend_from_slice(e);
        Ok(())
    }
    async fn purge(&self, _: &Limits) -> Result<()> {
        Ok(())
    }
    async fn report(&self, _: &nexofolio_audit_contracts::Filter) -> Result<serde_json::Value> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(serde_json::json!({"events":[],"buckets":[]}))
    }
}
struct Session {
    id: UserId,
    account: &'static str,
}
#[async_trait]
impl Sessions for Session {
    async fn verify_session(&self, token: &Secret) -> Result<SessionPrincipal> {
        if token.expose() != "valid" {
            return Err(Error::Unauthenticated);
        }
        Ok(SessionPrincipal {
            user_id: self.id,
            instance: "test".into(),
        })
    }
    async fn me(&self, _: &SessionPrincipal) -> Result<UserProfile> {
        Ok(UserProfile {
            id: self.id,
            account: self.account.into(),
            display_name: "fixture".into(),
            enabled: true,
            avatar_url: None,
        })
    }
}
fn settings() -> Settings {
    Settings::parse(&|_| None).unwrap()
}
fn event() -> Event {
    Event {
        execution: None,
        at: chrono::Utc::now(),
        kind: "api".into(),
        action: "request".into(),
        actor: "ip:127.0.0.1".into(),
        route: "/example".into(),
        method: "GET".into(),
        status: 200,
        duration_ms: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        project: String::new(),
        resource: String::new(),
        code: String::new(),
    }
}

#[tokio::test]
async fn only_fixed_verified_account_can_read_even_when_url_is_known() {
    for (account, token, status) in [
        ("a80646", Some("valid"), 200),
        ("A80646", Some("valid"), 200),
        ("A806460", Some("valid"), 403),
        ("another", Some("valid"), 403),
        ("a80646", None, 401),
        ("a80646", Some("a80646"), 401),
    ] {
        let store = Arc::new(Store::default());
        let app = routes::routes(
            Arc::new(Session {
                id: UserId::new(),
                account,
            }),
            store.clone(),
            None,
        );
        let mut req = Request::builder().uri("/v1/audit");
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(
            store.reads.load(Ordering::Relaxed),
            u64::from(status == 200)
        );
    }
}
#[test]
fn proxy_chain_requires_a_trusted_peer_and_stops_at_last_untrusted_hop() {
    use super::http::client_ip;
    let local = "127.0.0.1".parse().unwrap();
    let client = "203.0.113.4".parse().unwrap();
    assert_eq!(
        client_ip(Some(client), Some("1.2.3.4"), &[local]),
        Some(client)
    );
    assert_eq!(
        client_ip(Some(local), Some("1.2.3.4, 203.0.113.4"), &[local]),
        Some(client)
    );
    assert_eq!(
        client_ip(Some(local), Some("malformed"), &[local]),
        Some(local)
    );
    assert_eq!(client_ip(None, Some("1.2.3.4"), &[local]), None);
}
#[tokio::test]
async fn overflow_is_nonblocking_and_critical_events_have_reserved_capacity() {
    let store = Arc::new(Store::default());
    let stop = CancellationToken::new();
    let (audit, worker) = Audit::start(store.clone(), settings(), stop.clone());
    // No await: worker cannot consume on this single-thread test runtime.
    for _ in 0..5000 {
        audit.emit(event());
    }
    let mut critical = event();
    critical.kind = "operation".into();
    audit.emit(critical);
    assert_eq!(audit.health()["dropped_access"], 904);
    assert_eq!(audit.health()["dropped_important"], 0);
    stop.cancel();
    worker.await.unwrap();
    assert_eq!(store.events.lock().unwrap().len(), 4097);
}
#[tokio::test]
async fn identity_is_taken_from_auth_and_query_strings_never_enter_evidence() {
    let store = Arc::new(Store::default());
    let stop = CancellationToken::new();
    let (audit, worker) = Audit::start(store.clone(), settings(), stop.clone());
    let id = UserId::new();
    let sessions: Arc<dyn Sessions> = Arc::new(Session {
        id,
        account: "a80646",
    });
    let app = Router::new()
        .route(
            "/v1/example/{id}",
            get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        )
        .route_layer(middleware::from_fn_with_state(
            sessions,
            crate::http::access::session_auth,
        ))
        .layer(middleware::from_fn(
            move |mut req: axum::extract::Request, next: axum::middleware::Next| {
                let audit = audit.clone();
                async move {
                    req.extensions_mut()
                        .insert(crate::http::RequestId("request-fixture".into()));
                    super::observe(audit, req, next).await
                }
            },
        ));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/example/secret-path?token=private-query")
                .header("authorization", "Bearer valid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    stop.cancel();
    worker.await.unwrap();
    let events = store.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    for e in events.iter() {
        assert_eq!(e.actor, format!("user:{id}"));
        assert_eq!(e.route, "/v1/example/{id}");
        assert_eq!(e.request_id, "request-fixture");
    }
    let saved = serde_json::to_string(&*events).unwrap();
    assert!(!saved.contains("secret-path"));
    assert!(!saved.contains("private-query"));
    assert!(!saved.contains("Bearer"));
}
#[tokio::test]
async fn telemetry_rejects_spoofed_identity_and_does_not_log_payloads() {
    let store = Arc::new(Store::default());
    let stop = CancellationToken::new();
    let (audit, worker) = Audit::start(store.clone(), settings(), stop.clone());
    let id = UserId::new();
    let app = routes::routes(
        Arc::new(Session {
            id,
            account: "other",
        }),
        store.clone(),
        None,
    )
    .layer(Extension(audit));
    for (body, status) in [
        (r#"{"kind":"page","route":"home","actor":"a80646"}"#, 422),
        (
            r#"{"kind":"page","route":"https://secret?token=secret"}"#,
            400,
        ),
        (r#"{"kind":"page","route":"home"}"#, 204),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/telemetry")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer valid")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
    stop.cancel();
    worker.await.unwrap();
    let events = store.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor, format!("user:{id}"));
}

#[tokio::test]
#[ignore = "requires isolated TEST_AUDIT_DATABASE_URL"]
async fn postgres_rollups_caps_retention_and_filters() {
    use nexofolio_audit_adapter::PostgresAudit;
    use nexofolio_audit_contracts::Filter;
    let url = std::env::var("TEST_AUDIT_DATABASE_URL").expect("isolated audit database required");
    let db = PostgresAudit::new(&Secret::new(url)).unwrap();
    db.migrate().await.unwrap();
    db.migrate().await.unwrap();
    let limits = Limits {
        events: 2,
        buckets: 2,
        ..Limits::default()
    };
    let base = event();
    let mut operation = base.clone();
    operation.kind = "operation".into();
    operation.action = "curation.apply".into();
    db.write(&[base.clone(), base.clone(), operation.clone()], &limits)
        .await
        .unwrap();
    let query = Filter {
        limit: 200,
        ..Filter::default()
    };
    let data = db.report(&query).await.unwrap();
    assert_eq!(data["buckets"][0]["hits"], 2);
    assert_eq!(data["storage"]["events"], 1);
    assert_eq!(data["storage"]["buckets"], 1);
    let mut another = base.clone();
    another.actor = "ip:203.0.113.4".into();
    let mut third = base.clone();
    third.actor = "ip:203.0.113.5".into();
    db.write(
        &[another, third, operation.clone(), operation.clone()],
        &limits,
    )
    .await
    .unwrap();
    let data = db.report(&query).await.unwrap();
    assert_eq!(data["events"].as_array().unwrap().len(), 2);
    assert_eq!(data["buckets"].as_array().unwrap().len(), 2);
    assert_eq!(data["storage"]["evicted_events"], 1);
    assert_eq!(data["storage"]["evicted_buckets"], 1);
    let mut old = base.clone();
    old.at -= chrono::TimeDelta::days(120);
    old.actor = "ip:192.0.2.1".into();
    let mut old_error = old.clone();
    old_error.kind = "error".into();
    let large = Limits::default();
    db.write(&[old, old_error], &large).await.unwrap();
    db.purge(&large).await.unwrap();
    let data = db.report(&query).await.unwrap();
    assert_eq!(data["storage"]["events"], 2);
    assert_eq!(data["storage"]["buckets"], 2);
    let filtered = db
        .report(&Filter {
            request_id: Some(operation.request_id.clone()),
            kind: Some("operation".into()),
            limit: 1,
            ..Filter::default()
        })
        .await
        .unwrap();
    assert_eq!(filtered["events"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["events"][0]["action"], "curation.apply");
    db.close().await;
}

#[tokio::test]
async fn unavailable_storage_never_blocks_http_and_loss_is_visible() {
    struct Down;
    #[async_trait]
    impl AuditStore for Down {
        async fn write(&self, _: &[Event], _: &Limits) -> Result<()> {
            Err(Error::Unavailable { component: "audit" })
        }
        async fn purge(&self, _: &Limits) -> Result<()> {
            Ok(())
        }
        async fn report(&self, _: &nexofolio_audit_contracts::Filter) -> Result<serde_json::Value> {
            unreachable!()
        }
    }
    let stop = CancellationToken::new();
    let (audit, worker) = Audit::start(Arc::new(Down), settings(), stop.clone());
    let state = audit.clone();
    let app = Router::new()
        .route("/ok", get(|| async { StatusCode::NO_CONTENT }))
        .layer(middleware::from_fn(move |req, next| {
            let audit = state.clone();
            async move { super::observe(audit, req, next).await }
        }));
    let response = tokio::time::timeout(
        Duration::from_millis(100),
        app.oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 204);
    stop.cancel();
    worker.await.unwrap();
    assert_eq!(audit.health()["storage_failures"], 1);
    assert_eq!(audit.health()["dropped_access"], 1);
}
