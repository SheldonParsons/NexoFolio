//! Public release metadata. No project, session, database, or binary proxy dependency.
mod manifests;
#[cfg(test)]
mod tests;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    routing::get,
};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, watch},
    time::Instant,
};

const FETCHER_ROOT: &str = "https://asynctest.oss-cn-shenzhen.aliyuncs.com/nexofolio_fetcher/";
const INTERNAL_ROOT: &str = "https://asynctest.oss-cn-shenzhen.aliyuncs.com/core/updates/internal/";
const PUBLIC_ROOT: &str = "https://asynctest.oss-cn-shenzhen.aliyuncs.com/core/updates/public/";
const CACHE_TTL: Duration = Duration::from_secs(300);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum DesktopChannel {
    #[default]
    Internal,
    Public,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadQuery {
    #[serde(default)]
    desktop_channel: DesktopChannel,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReleaseStatus {
    Ready,
    Unpublished,
    Error,
}

#[derive(Clone, Debug, Serialize)]
struct ReleaseState<T> {
    status: ReleaseStatus,
    release: Option<T>,
}
impl<T> ReleaseState<T> {
    fn from_result(result: Result<T, ReleaseStatus>) -> Self {
        match result {
            Ok(release) => Self {
                status: ReleaseStatus::Ready,
                release: Some(release),
            },
            Err(status) => Self {
                status,
                release: None,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct DesktopRelease {
    version: String,
    url: String,
    filename: String,
    size: u64,
}

#[derive(Clone, Debug, Serialize)]
struct FetcherRelease {
    #[serde(flatten)]
    artifact: DesktopRelease,
    sha256: String,
    minimum_chrome_version: String,
    channel: String,
    installation: String,
}

#[derive(Serialize)]
struct DesktopDownload {
    platform: &'static str,
    #[serde(flatten)]
    state: ReleaseState<DesktopRelease>,
}
#[derive(Serialize)]
struct Downloads {
    schema_version: u32,
    desktop_channel: DesktopChannel,
    fetcher: ReleaseState<FetcherRelease>,
    desktop: [DesktopDownload; 3],
}

struct Cache<T> {
    value: Option<(Instant, ReleaseState<T>)>,
    flight: Option<watch::Receiver<Option<ReleaseState<T>>>>,
}
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            value: None,
            flight: None,
        }
    }
}

type Parser<T> = fn(&[u8], &Url) -> Result<T, ReleaseStatus>;
struct Source<T> {
    directory: Option<Url>,
    manifest: &'static str,
    parser: Parser<T>,
    client: Option<Client>,
    cache: Mutex<Cache<T>>,
    ttl: Duration,
    timeout: Duration,
}
impl<T: Clone + Send + Sync + 'static> Source<T> {
    fn new(
        directory: Option<Url>,
        manifest: &'static str,
        parser: Parser<T>,
        client: Option<Client>,
    ) -> Arc<Self> {
        Arc::new(Self {
            directory,
            manifest,
            parser,
            client,
            cache: Mutex::new(Cache::default()),
            ttl: CACHE_TTL,
            timeout: FETCH_TIMEOUT,
        })
    }

    async fn get(self: &Arc<Self>) -> ReleaseState<T> {
        let mut cache = self.cache.lock().await;
        if let Some((at, value)) = &cache.value
            && at.elapsed() < self.ttl
        {
            return value.clone();
        }
        let mut receiver = if let Some(receiver) = &cache.flight {
            receiver.clone()
        } else {
            let (sender, receiver) = watch::channel(None);
            cache.flight = Some(receiver.clone());
            let source = self.clone();
            // One bounded task per source. A disconnected browser cannot cancel a shared fetch.
            tokio::spawn(async move {
                let result = tokio::time::timeout(source.timeout, source.fetch())
                    .await
                    .unwrap_or(Err(ReleaseStatus::Error));
                let value = ReleaseState::from_result(result);
                let mut cache = source.cache.lock().await;
                cache.value = Some((Instant::now(), value.clone()));
                cache.flight = None;
                sender.send_replace(Some(value));
            });
            receiver
        };
        drop(cache);
        let result = receiver.wait_for(|value| value.is_some()).await;
        match result {
            Ok(value) => value.as_ref().expect("wait predicate").clone(),
            Err(_) => ReleaseState::from_result(Err(ReleaseStatus::Error)),
        }
    }

    async fn fetch(&self) -> Result<T, ReleaseStatus> {
        let directory = self.directory.as_ref().ok_or(ReleaseStatus::Error)?;
        let client = self.client.as_ref().ok_or(ReleaseStatus::Error)?;
        let url = directory
            .join(self.manifest)
            .map_err(|_| ReleaseStatus::Error)?;
        let mut response = client
            .get(url)
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| ReleaseStatus::Error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(ReleaseStatus::Unpublished);
        }
        // Redirects, partial responses, auth errors, and server errors are never interpreted as releases.
        if response.status() != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|n| n > MAX_MANIFEST_BYTES as u64)
        {
            return Err(ReleaseStatus::Error);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ReleaseStatus::Error)? {
            if bytes.len() + chunk.len() > MAX_MANIFEST_BYTES {
                return Err(ReleaseStatus::Error);
            }
            bytes.extend_from_slice(&chunk);
        }
        (self.parser)(&bytes, directory)
    }
}

struct DownloadService {
    fetcher: Arc<Source<FetcherRelease>>,
    internal: [Arc<Source<DesktopRelease>>; 3],
    public: [Arc<Source<DesktopRelease>>; 3],
}
impl DownloadService {
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let root = |key, default| {
            let value = lookup(key)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| String::from(default));
            let result = manifests::release_directory(&value);
            if result.is_none() {
                tracing::error!(setting = key, "invalid_download_release_directory");
            }
            result
        };
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(FETCH_TIMEOUT)
            .user_agent("NexoFolio-release-metadata/1")
            .build()
            .ok();
        if client.is_none() {
            tracing::error!("download_http_client_unavailable");
        }
        let desktop = |root: Option<Url>| {
            let source = |subdir, name, parser| {
                Source::new(
                    root.as_ref().and_then(|url| url.join(subdir).ok()),
                    name,
                    parser,
                    client.clone(),
                )
            };
            [
                source(
                    "win/x64/",
                    "latest.yml",
                    manifests::parse_windows as Parser<DesktopRelease>,
                ),
                source("mac/arm64/", "latest-mac.yml", manifests::parse_mac),
                source("mac/x64/", "latest-mac.yml", manifests::parse_mac),
            ]
        };
        Self {
            fetcher: Source::new(
                root("NEXOFOLIO_FETCHER_RELEASE_DIR", FETCHER_ROOT),
                "latest.json",
                manifests::parse_fetcher,
                client.clone(),
            ),
            internal: desktop(root(
                "NEXOFOLIO_DESKTOP_INTERNAL_RELEASE_DIR",
                INTERNAL_ROOT,
            )),
            public: desktop(root("NEXOFOLIO_DESKTOP_PUBLIC_RELEASE_DIR", PUBLIC_ROOT)),
        }
    }

    async fn get(&self, channel: DesktopChannel) -> Downloads {
        let sources = match channel {
            DesktopChannel::Internal => &self.internal,
            DesktopChannel::Public => &self.public,
        };
        let (fetcher, windows, arm, intel) = tokio::join!(
            self.fetcher.get(),
            sources[0].get(),
            sources[1].get(),
            sources[2].get()
        );
        Downloads {
            schema_version: 1,
            desktop_channel: channel,
            fetcher,
            desktop: [
                DesktopDownload {
                    platform: "windows",
                    state: windows,
                },
                DesktopDownload {
                    platform: "mac-arm64",
                    state: arm,
                },
                DesktopDownload {
                    platform: "mac-x64",
                    state: intel,
                },
            ],
        }
    }
}

async fn downloads(
    State(service): State<Arc<DownloadService>>,
    query: Result<Query<DownloadQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<
    ([(header::HeaderName, &'static str); 1], Json<Downloads>),
    (StatusCode, Json<serde_json::Value>),
> {
    let Query(query) = query.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"invalid_download_query"})),
        )
    })?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(service.get(query.desktop_channel).await),
    ))
}

fn routes_with(service: DownloadService) -> Router {
    Router::new()
        .route("/v1/downloads", get(downloads))
        .with_state(Arc::new(service))
}

/// Environment is sampled once per router construction; only seven fixed upstream cache slots exist.
pub fn routes() -> Router {
    routes_with(DownloadService::from_lookup(|key| std::env::var(key).ok()))
}
