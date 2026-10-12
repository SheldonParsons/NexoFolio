use super::*;

fn service_at(mock: &Mock) -> DownloadService {
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
            source.directory = Some(mock.root.join(&format!("{root}{subdir}")).unwrap());
            source.client = Some(client());
        }
    }
    let source = Arc::get_mut(&mut service.fetcher).unwrap();
    source.directory = Some(mock.root.join("fetcher/").unwrap());
    source.client = Some(client());
    service
}

async fn releases(version: Arc<AtomicUsize>) -> Mock {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().fallback(move |request: Request| {
        let counter = counter.clone();
        let version = version.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            if request.uri().path().ends_with("latest.json") {
                return Json(fetcher_json()).into_response();
            }
            let yaml = desktop_yaml()
                .replace("3.3.10", &format!("3.3.{}", version.load(Ordering::SeqCst)));
            (StatusCode::OK, yaml).into_response()
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Mock { root, calls, task }
}

async fn request(app: Router, method: &str, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn seven_links_redirect_get_and_head_and_share_the_metadata_cache() {
    let mock = releases(Arc::new(AtomicUsize::new(10))).await;
    let app = routes_with(service_at(&mock));
    let mut targets = vec![(
        "/v1/downloads/fetcher/latest".to_owned(),
        "fetcher/NexoFolio-Fetcher-0.1.0-dev.zip".to_owned(),
    )];
    for channel in ["internal", "public"] {
        for (platform, subdir, filename) in [
            ("windows", "win/x64", "AsyncTest%20Setup%203.3.10.exe"),
            ("mac-arm64", "mac/arm64", "AsyncTest-3.3.10.dmg"),
            ("mac-x64", "mac/x64", "AsyncTest-3.3.10.dmg"),
        ] {
            targets.push((
                format!("/v1/downloads/asynctest/{channel}/{platform}/latest"),
                format!("{channel}/{subdir}/{filename}"),
            ));
        }
    }
    for (index, (uri, target)) in targets.iter().enumerate() {
        for method in ["GET", "HEAD"] {
            let response = request(app.clone(), method, uri).await;
            assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{uri}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                response.headers()[header::LOCATION],
                mock.root.join(target).unwrap().as_str()
            );
            assert!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
        // Only the selected manifest was fetched; neither HEAD nor GET fetches any installer.
        assert_eq!(mock.calls.load(Ordering::SeqCst), index + 1);
    }
    for channel in ["internal", "public"] {
        let (status, data) = response(
            app.clone(),
            &format!("/v1/downloads?desktop_channel={channel}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data["fetcher"]["status"], "ready");
        assert!(
            data["desktop"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["status"] == "ready")
        );
    }
    assert_eq!(mock.calls.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn fixed_link_resolves_new_version_after_shared_cache_expiry() {
    let version = Arc::new(AtomicUsize::new(10));
    let mock = releases(version.clone()).await;
    let service = service_at(&mock);
    let source = service.public[0].clone();
    let app = routes_with(service);
    let uri = "/v1/downloads/asynctest/public/windows/latest";
    let (_, data) = response(app.clone(), "/v1/downloads?desktop_channel=public").await;
    assert_eq!(data["desktop"][0]["release"]["version"], "3.3.10");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 4);
    version.store(11, Ordering::SeqCst);
    let cached = request(app.clone(), "GET", uri).await;
    assert_eq!(
        cached.headers()[header::LOCATION],
        data["desktop"][0]["release"]["url"].as_str().unwrap()
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 4);
    {
        let mut cache = source.cache.lock().await;
        cache.value.as_mut().unwrap().0 = Instant::now() - CACHE_TTL;
    }
    let refreshed = request(app.clone(), "GET", uri).await;
    assert_eq!(refreshed.status(), StatusCode::TEMPORARY_REDIRECT);
    assert!(
        refreshed.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .ends_with("AsyncTest%20Setup%203.3.11.exe")
    );
    let (_, data) = response(app, "/v1/downloads?desktop_channel=public").await;
    assert_eq!(data["desktop"][0]["release"]["version"], "3.3.11");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn invalid_selectors_and_queries_never_fetch_or_redirect() {
    let mock = releases(Arc::new(AtomicUsize::new(10))).await;
    let app = routes_with(service_at(&mock));
    for uri in [
        "/v1/downloads/asynctest/other/windows/latest",
        "/v1/downloads/asynctest/public/linux/latest",
        "/v1/downloads/asynctest/internal/mac-arm64%0A/latest",
    ] {
        let result = request(app.clone(), "GET", uri).await;
        assert_eq!(result.status(), StatusCode::NOT_FOUND);
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!result.headers().contains_key(header::LOCATION));
    }
    for uri in [
        "/v1/downloads/fetcher/latest?url=https://evil.test",
        "/v1/downloads/fetcher/latest?desktop_channel=public",
        "/v1/downloads/asynctest/public/windows/latest?channel=internal",
        "/v1/downloads/asynctest/internal/mac-arm64/latest?refresh=true",
    ] {
        let result = request(app.clone(), "GET", uri).await;
        assert_eq!(result.status(), StatusCode::BAD_REQUEST);
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!result.headers().contains_key(header::LOCATION));
    }
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unpublished_or_invalid_release_returns_error_without_download_fallback() {
    for (status, body, expected_status, code) in [
        (
            StatusCode::NOT_FOUND,
            "",
            StatusCode::NOT_FOUND,
            "download_unpublished",
        ),
        (
            StatusCode::BAD_GATEWAY,
            "private diagnostics",
            StatusCode::SERVICE_UNAVAILABLE,
            "download_unavailable",
        ),
        (
            StatusCode::OK,
            "version: invalid",
            StatusCode::SERVICE_UNAVAILABLE,
            "download_unavailable",
        ),
    ] {
        let mock = mock(status, body.into(), Duration::ZERO).await;
        let app = routes_with(service_at(&mock));
        for uri in [
            "/v1/downloads/fetcher/latest",
            "/v1/downloads/asynctest/internal/windows/latest",
        ] {
            let result = request(app.clone(), "GET", uri).await;
            assert_eq!(result.status(), expected_status);
            assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!result.headers().contains_key(header::LOCATION));
            let bytes = result.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error":code})
            );
        }
        assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    }
}
