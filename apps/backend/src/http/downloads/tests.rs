use super::*;
use axum::{body::Body, extract::Request, response::IntoResponse};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{io::AsyncWriteExt, net::TcpListener};
use tower::ServiceExt;

fn fetcher_json() -> Value {
    json!({"schemaVersion":1,"product":"nexofolio-fetcher","channel":"development","version":"0.1.0","installation":"load-unpacked","minimumChromeVersion":"125","artifact":{"filename":"NexoFolio-Fetcher-0.1.0-dev.zip","url":"https://old.invalid/nexofolio-fetcher/old.zip","size":176441,"sha256":"a".repeat(64)}})
}
fn desktop_yaml() -> &'static str {
    "version: 3.3.10\nfiles:\n  - url: AsyncTest-3.3.10-mac.zip\n    size: 456\n  - url: AsyncTest-3.3.10.dmg\n    size: 789\n  - url: AsyncTest Setup 3.3.10.exe\n    size: 123\nreleaseNotes: 'unknown fields are permitted'\n"
}
fn directory() -> Url {
    Url::parse(INTERNAL_ROOT).unwrap()
}

#[test]
fn fetcher_repairs_old_location_and_keeps_installation_metadata() {
    let result = manifests::parse_fetcher(
        &serde_json::to_vec(&fetcher_json()).unwrap(),
        &Url::parse(FETCHER_ROOT).unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.artifact.url,
        format!("{FETCHER_ROOT}NexoFolio-Fetcher-0.1.0-dev.zip")
    );
    assert_eq!(result.minimum_chrome_version, "125");
    assert_eq!(result.installation, "load-unpacked");
    assert_eq!(result.channel, "development");
    let serialized = serde_json::to_value(result).unwrap();
    assert_eq!(serialized["version"], "0.1.0");
    assert!(serialized.get("artifact").is_none());
}

#[test]
fn fetcher_rejects_invalid_contract_and_unsafe_filenames() {
    for (pointer, value) in [
        ("/schemaVersion", json!(2)),
        ("/product", json!("other")),
        ("/channel", json!("public")),
        ("/installation", json!("store")),
        ("/version", json!("latest")),
        ("/minimumChromeVersion", json!(125)),
        ("/minimumChromeVersion", json!("125..0")),
        ("/artifact/size", json!(0)),
        ("/artifact/size", json!(9007199254740992_u64)),
        ("/artifact/sha256", json!("xyz")),
        (
            "/artifact/filename",
            json!("../NexoFolio-Fetcher-0.1.0-dev.zip"),
        ),
    ] {
        let mut value_json = fetcher_json();
        *value_json.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            manifests::parse_fetcher(&serde_json::to_vec(&value_json).unwrap(), &directory())
                .unwrap_err(),
            ReleaseStatus::Error,
            "{pointer}"
        );
    }
}

#[test]
fn desktop_selects_installers_and_preserves_channel_directories() {
    for root in [INTERNAL_ROOT, PUBLIC_ROOT] {
        let mac = Url::parse(root).unwrap().join("mac/arm64/").unwrap();
        let result = manifests::parse_mac(desktop_yaml().as_bytes(), &mac).unwrap();
        assert_eq!(result.url, format!("{root}mac/arm64/AsyncTest-3.3.10.dmg"));
        assert_eq!(result.size, 789);
        let win = manifests::parse_windows(desktop_yaml().as_bytes(), &directory()).unwrap();
        assert!(win.url.ends_with("AsyncTest%20Setup%203.3.10.exe"));
        assert_eq!(win.filename, "AsyncTest Setup 3.3.10.exe");
    }
    assert_eq!(
        manifests::parse_mac(b"version: 3.3.10\nfiles: []", &directory()).unwrap_err(),
        ReleaseStatus::Unpublished
    );
}

#[test]
fn desktop_rejects_path_escape_and_malformed_yaml() {
    for filename in [
        "../evil.dmg",
        "https://evil.test/a.dmg",
        "//evil.test/a.dmg",
        "a%2f..%2fb.dmg",
        "a\\b.dmg",
        "./a.dmg",
        "a?b.dmg",
        "a#b.dmg",
        "nested/a.dmg",
    ] {
        let text = format!("version: 3.3.10\nfiles: [{{url: '{filename}', size: 123}}]");
        assert_eq!(
            manifests::parse_mac(text.as_bytes(), &directory()).unwrap_err(),
            ReleaseStatus::Error,
            "{filename}"
        );
    }
    for text in [
        "version: 3.3.10\nversion: 3.3.11\nfiles: []",
        "version: 3.3.10\nfiles: &files []\nx: *files",
        "version: 3.3.10\nfiles: []\n---\nversion: 4.0.0\nfiles: []",
        "version: invalid\nfiles: []",
        "version: 3.3.10",
        "files: []",
        "version: 3.3.10\nfiles: [{url: a.dmg, size: -1}]",
        "version: 3.3.10\nfiles: [{url: a.dmg, size: 0}]",
        "version: 3.3.10\nfiles: [{url: a.dmg, size: 1.5}]",
    ] {
        assert_eq!(
            manifests::parse_mac(text.as_bytes(), &directory()).unwrap_err(),
            ReleaseStatus::Error,
            "{text}"
        );
    }
    let deep = format!(
        "version: 3.3.10\nfiles: []\nunknown: {}0{}",
        "[".repeat(100),
        "]".repeat(100)
    );
    assert!(manifests::parse_mac(deep.as_bytes(), &directory()).is_err());
}

#[test]
fn configuration_only_accepts_public_https_directory_shape() {
    for bad in [
        "http://example.com/",
        "file:///etc/",
        "https://user:secret@example.com/",
        "https://example.com/?token=secret",
        "https://example.com/#fragment",
    ] {
        assert!(manifests::release_directory(bad).is_none());
    }
    assert_eq!(
        manifests::release_directory("https://example.com/releases")
            .unwrap()
            .as_str(),
        "https://example.com/releases/"
    );
    let service = DownloadService::from_lookup(|key| {
        (key == "NEXOFOLIO_DESKTOP_INTERNAL_RELEASE_DIR").then(|| "bad".into())
    });
    assert!(service.internal[0].directory.is_none());
    assert!(service.public[0].directory.is_some());
}

struct Mock {
    root: Url,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn mock(status: StatusCode, body: String, delay: Duration) -> Mock {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().fallback(move || {
        let counter = counter.clone();
        let body = body.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(delay).await;
            (status, body)
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock { root, calls, task }
}
fn client() -> Client {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}
fn source(mock: &Mock) -> Arc<Source<DesktopRelease>> {
    Source::new(
        Some(mock.root.clone()),
        "latest.yml",
        manifests::parse_windows,
        Some(client()),
    )
}

#[tokio::test]
async fn concurrent_reads_are_deduplicated_and_cache_expires() {
    let mock = mock(
        StatusCode::OK,
        desktop_yaml().into(),
        Duration::from_millis(20),
    )
    .await;
    let mut source = source(&mock);
    Arc::get_mut(&mut source).unwrap().ttl = Duration::from_millis(30);
    let mut jobs = Vec::new();
    for _ in 0..20 {
        let source = source.clone();
        jobs.push(tokio::spawn(async move { source.get().await }));
    }
    for job in jobs {
        assert_eq!(job.await.unwrap().status, ReleaseStatus::Ready);
    }
    assert_eq!(source.get().await.status, ReleaseStatus::Ready);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(source.get().await.status, ReleaseStatus::Ready);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_waiter_does_not_restart_shared_fetch() {
    let mock = mock(
        StatusCode::OK,
        desktop_yaml().into(),
        Duration::from_millis(80),
    )
    .await;
    let source = source(&mock);
    let copy = source.clone();
    let waiter = tokio::spawn(async move { copy.get().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while mock.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    assert_eq!(source.get().await.status, ReleaseStatus::Ready);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failures_are_independent_and_cached_including_timeouts_and_body_limit() {
    for (status, body, delay, expected) in [
        (
            StatusCode::NOT_FOUND,
            "".into(),
            Duration::ZERO,
            ReleaseStatus::Unpublished,
        ),
        (
            StatusCode::OK,
            "version: 1.2.3\nfiles: []".into(),
            Duration::ZERO,
            ReleaseStatus::Unpublished,
        ),
        (
            StatusCode::FORBIDDEN,
            "secret diagnostic".into(),
            Duration::ZERO,
            ReleaseStatus::Error,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "".into(),
            Duration::ZERO,
            ReleaseStatus::Error,
        ),
        (
            StatusCode::FOUND,
            "".into(),
            Duration::ZERO,
            ReleaseStatus::Error,
        ),
        (
            StatusCode::OK,
            "invalid".into(),
            Duration::ZERO,
            ReleaseStatus::Error,
        ),
        (
            StatusCode::OK,
            "x".repeat(MAX_MANIFEST_BYTES + 1),
            Duration::ZERO,
            ReleaseStatus::Error,
        ),
        (
            StatusCode::OK,
            desktop_yaml().into(),
            Duration::from_millis(150),
            ReleaseStatus::Error,
        ),
    ] {
        let mock = mock(status, body, delay).await;
        let mut source = source(&mock);
        Arc::get_mut(&mut source).unwrap().timeout = Duration::from_millis(60);
        for _ in 0..2 {
            let result = source.get().await;
            assert_eq!(result.status, expected);
            assert!(result.release.is_none());
        }
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn streaming_body_without_content_length_is_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        let bytes = "x".repeat(MAX_MANIFEST_BYTES + 1);
        let _ = socket
            .write_all(format!("{:x}\r\n{bytes}\r\n0\r\n\r\n", bytes.len()).as_bytes())
            .await;
    });
    let source = Source::new(
        Some(root),
        "latest.yml",
        manifests::parse_windows,
        Some(client()),
    );
    assert_eq!(source.get().await.status, ReleaseStatus::Error);
    task.await.unwrap();
}

async fn response(app: Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn public_route_contract_defaults_isolates_channels_and_rejects_proxy_queries() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let upstream = Router::new().fallback(move |request: Request| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            let path = request.uri().path();
            if path.ends_with("latest.json") {
                return Json(fetcher_json()).into_response();
            }
            if path.contains("internal/win") {
                return StatusCode::NOT_FOUND.into_response();
            }
            if path.contains("internal/mac/x64") {
                return (StatusCode::OK, "bad yaml").into_response();
            }
            (StatusCode::OK, desktop_yaml()).into_response()
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let mut service = DownloadService::from_lookup(|_| None);
    for (root, sources) in [
        ("internal/", &mut service.internal),
        ("public/", &mut service.public),
    ] {
        for (subdir, source) in ["win/x64/", "mac/arm64/", "mac/x64/"]
            .into_iter()
            .zip(sources)
        {
            let source = Arc::get_mut(source).unwrap();
            source.directory = Some(base.join(&format!("{root}{subdir}")).unwrap());
            source.client = Some(client());
        }
    }
    let fetcher = Arc::get_mut(&mut service.fetcher).unwrap();
    fetcher.directory = Some(base.join("fetcher/").unwrap());
    fetcher.client = Some(client());
    let app = routes_with(service);
    let (status, data) = response(app.clone(), "/v1/downloads").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["desktop_channel"], "internal");
    assert_eq!(data["fetcher"]["release"]["channel"], "development");
    assert_eq!(
        data["desktop"][0],
        json!({"platform":"windows","status":"unpublished","release":null})
    );
    assert_eq!(data["desktop"][1]["status"], "ready");
    assert!(
        data["desktop"][1]["release"]["url"]
            .as_str()
            .unwrap()
            .contains("/internal/mac/arm64/")
    );
    assert_eq!(
        data["desktop"][2],
        json!({"platform":"mac-x64","status":"error","release":null})
    );
    let (status, public) = response(app.clone(), "/v1/downloads?desktop_channel=public").await;
    assert_eq!(status, StatusCode::OK);
    for item in public["desktop"].as_array().unwrap() {
        assert_eq!(item["status"], "ready");
        assert!(
            item["release"]["url"]
                .as_str()
                .unwrap()
                .contains("/public/")
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    for uri in [
        "/v1/downloads?desktop_channel=other",
        "/v1/downloads?url=https://evil.test",
        "/v1/downloads?refresh=true",
        "/v1/downloads?desktop_channel=internal&desktop_channel=public",
    ] {
        let (status, error) = response(app.clone(), uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error, json!({"error":"invalid_download_query"}));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    task.abort();
}

#[tokio::test]
async fn route_is_present_without_access_or_database_availability() {
    let config = crate::wiring::Config::from_lookup(|key| {
        (key == "DATABASE_URL").then(|| "postgres://unused/unused".into())
    })
    .unwrap();
    struct Offline;
    #[async_trait::async_trait]
    impl nexofolio_common::DatabaseProbe for Offline {
        async fn check(&self) -> nexofolio_common::Result<()> {
            panic!("downloads must not consult the database")
        }
    }
    let app = super::super::router(
        &config,
        Arc::new(Offline),
        Arc::new(nexofolio_access_adapter::Unconfigured),
        tokio_util::sync::CancellationToken::new(),
    );
    // Invalid query validates route mounting without reaching external services.
    let (status, data) = response(app, "/v1/downloads?desktop_channel=invalid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(data, json!({"error":"invalid_download_query"}));
}
