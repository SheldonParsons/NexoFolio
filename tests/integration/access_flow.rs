use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use nexofolio_access_adapter::{Postgres, PostgresAccess, Unconfigured};
use nexofolio_access_contracts::{
    ExternalProject, LoginStore, ProjectAccess, ProjectSnapshot, SessionPrincipal,
};
use nexofolio_backend::{
    http,
    wiring::{Config, Databases, build_api},
};
use nexofolio_common::{ProjectId, Secret, UserId};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Fixture {
    a: Arc<RwLock<Vec<Value>>>,
    b: Arc<RwLock<Vec<Value>>>,
    broken: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    logins: Arc<AtomicUsize>,
}
async fn tokens(State(s): State<Fixture>, Json(body): Json<Value>) -> axum::response::Response {
    s.calls.fetch_add(1, Ordering::SeqCst);
    s.logins.fetch_add(1, Ordering::SeqCst);
    if body["password"] != "fixture-pass" {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error":"invalid"}))).into_response();
    }
    let a = body["account"].as_str().unwrap_or("").to_lowercase();
    if !["alice", "bob"].contains(&a.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (StatusCode::CREATED, Json(json!({"token":a}))).into_response()
}
async fn user(State(s): State<Fixture>, h: HeaderMap) -> axum::response::Response {
    s.calls.fetch_add(1, Ordering::SeqCst);
    let a = h.get("Token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !["alice", "bob"].contains(&a) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rows = if a == "alice" {
        s.a.read().await.clone()
    } else {
        s.b.read().await.clone()
    };
    let ids = rows
        .iter()
        .map(|v| v["id"].to_string())
        .collect::<Vec<_>>()
        .join(",");
    Json(json!({"profile":{"id":if a=="alice"{1}else{2},"account":a,"realname":a,"deleted":"0","view":{"projects":ids}}})).into_response()
}
#[derive(Deserialize)]
struct Page {
    page: usize,
    status: String,
}
async fn projects(
    State(s): State<Fixture>,
    h: HeaderMap,
    Query(q): Query<Page>,
) -> axum::response::Response {
    s.calls.fetch_add(1, Ordering::SeqCst);
    assert_eq!(q.status, "all");
    if s.broken.load(Ordering::SeqCst) && q.page == 2 {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let a = h.get("Token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !["alice", "bob"].contains(&a) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rows = if a == "alice" {
        s.a.read().await.clone()
    } else {
        s.b.read().await.clone()
    };
    let batch: Vec<_> = rows
        .iter()
        .skip((q.page - 1) * 2)
        .take(2)
        .cloned()
        .collect();
    Json(json!({"page":q.page,"limit":2,"total":rows.len(),"projects":batch})).into_response()
}
async fn login(c: &reqwest::Client, url: &str, account: &str, password: &str) -> Value {
    let r = c
        .post(format!("{url}/v1/auth/login"))
        .json(&json!({"account":account,"password":password}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    r.json().await.unwrap()
}
async fn list(c: &reqwest::Client, url: &str, token: &str) -> Value {
    let r = c
        .get(format!("{url}/v1/projects"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn real_database_http_login_sync_permissions_and_token_lifecycle() {
    let db_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let database = Postgres::new(&Secret::new(&db_url), 10, Duration::from_secs(2)).unwrap();
    database.migrate().await.unwrap();
    let sql = sqlx::PgPool::connect(&db_url).await.unwrap();
    let fixture = Fixture {
        a: Arc::new(RwLock::new(vec![
            json!({"id":11,"name":"A","status":"doing"}),
            json!({"id":12,"name":"Closed","status":"closed"}),
            json!({"id":13,"name":"Waiting","status":"wait"}),
        ])),
        b: Arc::new(RwLock::new(vec![
            json!({"id":21,"name":"B","status":"doing"}),
        ])),
        broken: Arc::new(AtomicBool::new(false)),
        calls: Arc::new(AtomicUsize::new(0)),
        logins: Arc::new(AtomicUsize::new(0)),
    };
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api.php/v1", upstream.local_addr().unwrap());
    let upstream_stop = CancellationToken::new();
    let cancellation = upstream_stop.clone();
    let remote_router = Router::new()
        .route("/api.php/v1/tokens", post(tokens))
        .route("/api.php/v1/user", get(user))
        .route("/api.php/v1/projects", get(projects))
        .with_state(fixture.clone());
    let remote = tokio::spawn(async move {
        axum::serve(upstream, remote_router)
            .with_graceful_shutdown(cancellation.cancelled_owned())
            .await
            .unwrap()
    });
    let emergency = Argon2::default()
        .hash_password(b"fixture-emergency", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string();
    let key = "ab".repeat(32);
    let config = Config::from_lookup(|name| match name {
        "DATABASE_URL" => Some(db_url.clone()),
        "NEXOFOLIO_ZENTAO_BASE_URL" => Some(base.clone()),
        "NEXOFOLIO_SESSION_KEY" => Some(key.clone()),
        "NEXOFOLIO_EMERGENCY_PASSWORD_HASH" => Some(emergency.clone()),
        "NEXOFOLIO_COLLECT_RATE_PER_MINUTE" => Some("1".into()),
        "NEXOFOLIO_COLLECT_BURST" => Some("8".into()),
        _ => None,
    })
    .unwrap();
    let databases = Databases::new(&config).unwrap();
    databases.migrate().await.unwrap();
    let access = build_api(&config, &databases).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let app = http::router_with_access(
        &config,
        Arc::new(database.clone()),
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
    let first = login(&client, &url, "ALICE", "fixture-pass").await;
    let token = first["token"].as_str().unwrap().to_owned();
    assert_eq!(first["token_reused"], false);
    assert_eq!(first["project_sync"]["status"], "completed");
    assert_eq!(first["project_sync"]["counts"]["created"], 1);
    let uid: Uuid = first["user"]["id"].as_str().unwrap().parse().unwrap();
    let row=sqlx::query("SELECT token_encrypted,token_hash,abs(extract(epoch FROM (expires_at-(issued_at+interval '3 months'))))::float8 AS delta FROM access.internal_sessions WHERE user_id=$1").bind(uid).fetch_one(&sql).await.unwrap();
    assert!(row.get::<f64, _>("delta") < 1.0);
    assert!(
        !row.get::<Vec<u8>, _>("token_encrypted")
            .windows(token.len())
            .any(|v| v == token.as_bytes())
    );
    assert_eq!(row.get::<Vec<u8>, _>("token_hash").len(), 32);
    let repeat = login(&client, &url, "alice", "fixture-pass").await;
    assert_eq!(repeat["token"], first["token"]);
    assert_eq!(repeat["expires_at"], first["expires_at"]);
    assert_eq!(repeat["token_reused"], true);
    let (one, two) = tokio::join!(
        login(&client, &url, "alice", "fixture-pass"),
        login(&client, &url, "ALICE", "fixture-pass")
    );
    assert_eq!(one["token"], first["token"]);
    assert_eq!(two["token"], first["token"]);
    let bad = client
        .post(format!("{url}/v1/auth/login"))
        .json(&json!({"account":"alice","password":"wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);
    let bob = login(&client, &url, "bob", "fixture-pass").await;
    let items = list(&client, &url, &token).await;
    assert_eq!(items["total"], 2);
    let cards = items["items"].as_array().unwrap();
    let a = cards.iter().find(|x| x["name"] == "A").unwrap();
    let b = cards.iter().find(|x| x["name"] == "B").unwrap();
    assert_eq!(a["can_access"], true);
    assert_eq!(b["can_access"], false);
    assert_eq!(b.as_object().unwrap().len(), 6);
    let a_id = a["project_id"].as_str().unwrap().to_owned();
    let b_id = b["project_id"].as_str().unwrap().to_owned();
    assert_eq!(
        client
            .get(format!("{url}/v1/projects/{b_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/projects/{a_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    sites_and_collect(&client, &url, &token, &a_id, &b_id, &sql).await;
    let before = fixture.calls.load(Ordering::SeqCst);
    let em = login(&client, &url, "ALICE", "fixture-emergency").await;
    assert_eq!(em["token"], first["token"]);
    assert_eq!(em["project_sync"]["status"], "skipped");
    assert_eq!(before, fixture.calls.load(Ordering::SeqCst));
    let unknown = client
        .post(format!("{url}/v1/auth/login"))
        .json(&json!({"account":"not-existing","password":"fixture-emergency"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 401);
    // Partial second-page failure must preserve both old permissions and project state.
    fixture.a.write().await[0]["status"] = json!("closed");
    fixture.broken.store(true, Ordering::SeqCst);
    let failed = login(&client, &url, "alice", "fixture-pass").await;
    assert_eq!(failed["token"], first["token"]);
    assert_eq!(failed["project_sync"]["status"], "failed");
    let v = list(&client, &url, &token).await;
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "A" && x["status"] == "doing" && x["can_access"] == true)
    );
    fixture.broken.store(false, Ordering::SeqCst);
    login(&client, &url, "alice", "fixture-pass").await;
    let v = list(&client, &url, &token).await;
    assert_eq!(v["total"], 2);
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "A" && x["status"] == "closed")
    );
    // Complete empty user scope removes that user's access, never the shared catalog.
    fixture.a.write().await.clear();
    login(&client, &url, "alice", "fixture-pass").await;
    assert_eq!(
        client
            .get(format!("{url}/v1/projects/{a_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(list(&client, &url, &token).await["total"], 2);
    // Local requests work without any remote dependency.
    upstream_stop.cancel();
    remote.await.unwrap();
    assert_eq!(
        client
            .get(format!("{url}/v1/auth/me"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(list(&client, &url, &token).await["total"], 2);
    assert_eq!(
        client
            .get(format!("{url}/mcp"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // Force expiry in the isolated test database, then recreate via emergency login.
    sqlx::query("UPDATE access.internal_sessions SET issued_at=now()-interval '5 months',expires_at=now()-interval '1 second' WHERE user_id=$1").bind(uid).execute(&sql).await.unwrap();
    assert_eq!(
        client
            .get(format!("{url}/v1/projects"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let renewed = login(&client, &url, "alice", "fixture-emergency").await;
    assert_ne!(renewed["token"], first["token"]);
    assert_eq!(renewed["token_reused"], false);
    let principal = SessionPrincipal {
        user_id: first["user"]["id"]
            .as_str()
            .unwrap()
            .parse::<UserId>()
            .unwrap(),
        instance: base.clone(),
    };
    let store = PostgresAccess::new(database.clone(), &Secret::new(&key)).unwrap();
    let newer = store.next_sync_generation().await.unwrap();
    store
        .apply_snapshot(
            &principal,
            newer,
            ProjectSnapshot {
                projects: vec![ExternalProject {
                    instance: base.clone(),
                    external_id: "11".into(),
                    name: "ignored-new-name".into(),
                    state: "doing".into(),
                }],
                visible_project_ids: vec!["11".into()],
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .apply_snapshot(
                &principal,
                newer - 1,
                ProjectSnapshot {
                    projects: vec![],
                    visible_project_ids: vec![]
                }
            )
            .await
            .is_err()
    );
    assert!(
        store
            .require_project(&principal, a_id.parse::<ProjectId>().unwrap())
            .await
            .is_ok()
    );
    let bob_token = bob["token"].as_str().unwrap();
    let bob_uid: Uuid = bob["user"]["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE access.users SET enabled=false WHERE id=$1")
        .bind(bob_uid)
        .execute(&sql)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{url}/v1/projects"))
            .bearer_auth(bob_token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // Removal is real even with configured login and a valid session.
    for path in [
        "/v1/ingestion/batches",
        "/v1/ingestion/capabilities",
        "/v1/projects/unused/evidence",
        "/v1/projects/unused/maintenance-runs",
    ] {
        for method in [reqwest::Method::GET, reqwest::Method::POST] {
            assert_eq!(
                client
                    .request(method, format!("{url}{path}"))
                    .bearer_auth(renewed["token"].as_str().unwrap())
                    .send()
                    .await
                    .unwrap()
                    .status(),
                404
            );
        }
    }
    stop.cancel();
    handle.await.unwrap().unwrap();
    database.close().await;
    databases.close().await;
    sql.close().await;
}

/// Site registry and collect on top of a logged-in user who can open project
/// `a` but not `b`.
async fn sites_and_collect(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    a: &str,
    b: &str,
    sql: &sqlx::PgPool,
) {
    let page = "https://shop.example.com/app/#/order/1001";
    let lookup = || async {
        let r = client
            .get(format!("{url}/v1/sites/lookup"))
            .query(&[("url", page)])
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["cache-control"], "no-store");
        r.json::<Value>().await.unwrap()
    };
    assert_eq!(
        lookup().await,
        Value::Null,
        "nothing bound yet, no login needed"
    );

    let environment: Value = client
        .post(format!("{url}/v1/projects/{a}/environments"))
        .bearer_auth(token)
        .json(&json!({"name": "test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let environment = environment["id"].as_str().unwrap().to_owned();
    let bind = |project: &str, environment: &str, token: Option<&str>| {
        let mut request = client.put(format!("{url}/v1/sites")).json(&json!({
            "site": {"origin": "https://shop.example.com", "prefix": "/app"},
            "project_id": project,
            "environment_id": environment,
        }));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send()
    };
    assert_eq!(bind(a, &environment, None).await.unwrap().status(), 401);
    assert_eq!(
        bind(b, &environment, Some(token)).await.unwrap().status(),
        403
    );
    let unknown = bind(a, &uuid::Uuid::new_v4().to_string(), Some(token))
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        unknown.json::<Value>().await.unwrap()["error"]["code"],
        "UNKNOWN_ENVIRONMENT"
    );
    assert_eq!(
        bind(a, &environment, Some(token)).await.unwrap().status(),
        204
    );
    assert_eq!(
        lookup().await,
        json!({
            "site": {"origin": "https://shop.example.com", "prefix": "/app"},
            "project": {"id": a, "name": "A"},
            "environment": {"id": environment, "name": "test"},
        })
    );

    let post = |body: Vec<u8>| {
        client
            .post(format!("{url}/v1/collect/batches"))
            .header("content-type", "application/json")
            .body(body)
            .send()
    };
    let batch = |project: &str| {
        json!({
            "batch_id": uuid::Uuid::new_v4(),
            "platform": "nexofolio-fetcher",
            "target": {"project_id": project, "environment": {"id": environment}},
            "records": [{
                "id": uuid::Uuid::new_v4(),
                "kind": "http_exchange",
                "version": 1,
                "observed_at": "2026-09-24T08:00:00Z",
                "payload": {
                    "request": {"method": "GET", "url": "https://shop.example.com/api/order/1001", "body": {"state": "none"}},
                    "response": {"status": 200, "body": {"state": "none"}}
                }
            }, {"id": "not-a-uuid", "kind": "http_exchange", "version": 1}]
        })
    };
    let refused = |expected: u16, code: &'static str| {
        move |r: reqwest::Response| async move {
            assert_eq!(r.status(), expected);
            assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], code);
        }
    };

    let first = batch(a);
    let r = post(serde_json::to_vec(&first).unwrap()).await.unwrap();
    assert_eq!(r.status(), 200);
    let receipt: Value = r.json().await.unwrap();
    assert_eq!(receipt["accepted"], 1);
    assert_eq!(receipt["rejected"][0]["reason"], "INVALID_RECORD");
    let retry = post(serde_json::to_vec_pretty(&first).unwrap())
        .await
        .unwrap();
    assert_eq!(
        retry.json::<Value>().await.unwrap(),
        receipt,
        "a retry gets the first receipt"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM intake.batches WHERE batch_id=$1")
        .bind(
            first["batch_id"]
                .as_str()
                .unwrap()
                .parse::<uuid::Uuid>()
                .unwrap(),
        )
        .fetch_one(sql)
        .await
        .unwrap();
    assert_eq!(stored, 1);

    let mut changed = first.clone();
    changed["platform_version"] = json!("2.0.0");
    refused(409, "BATCH_ID_REUSED")(post(serde_json::to_vec(&changed).unwrap()).await.unwrap())
        .await;
    refused(404, "UNKNOWN_PROJECT")(
        post(serde_json::to_vec(&batch(&uuid::Uuid::new_v4().to_string())).unwrap())
            .await
            .unwrap(),
    )
    .await;
    refused(400, "INVALID_BATCH")(post(b"{".to_vec()).await.unwrap()).await;
    refused(413, "BATCH_TOO_LARGE")(post(vec![b' '; 8 * 1024 * 1024 + 1]).await.unwrap()).await;

    // Burst of 8 per platform; the calls above used 5.
    let mut limited = None;
    for _ in 0..8 {
        let r = post(serde_json::to_vec(&batch(a)).unwrap()).await.unwrap();
        if r.status() == 429 {
            limited = Some(r);
            break;
        }
        assert_eq!(r.status(), 200);
    }
    let limited = limited.expect("rate limited within the burst");
    assert!(
        limited.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    refused(429, "RATE_LIMITED")(limited).await;
}
use uuid::Uuid;
