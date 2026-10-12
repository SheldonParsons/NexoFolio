//! OSS-only admission gate for the Fetcher runtime. Never reinterpret historical platform_version.
use axum::{
    Json,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use nexofolio_intake_adapter::PostgresLedger;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const ROOT: &str = "https://asynctest.oss-cn-shenzhen.aliyuncs.com/nexofolio_fetcher/";
const RUNTIME_HEADER: &str = "x-nexofolio-fetcher-version";
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    product: String,
    channel: String,
    installation: String,
    version: String,
    built_at: String,
    minimum_chrome_version: String,
    artifact: Artifact,
    policy_version: Option<u32>,
    policy_revision: Option<i64>,
    minimum_version: Option<String>,
    release_mode: Option<String>,
    release_notes: Option<Vec<String>>,
}
#[derive(Clone, Deserialize)]
struct Artifact {
    filename: String,
    url: String,
    size: u64,
    sha256: String,
}
#[derive(Clone)]
struct Policy {
    minimum: [i32; 4],
}
#[derive(Default)]
struct Cache {
    result: Option<Result<Policy, ()>>,
    until: Option<tokio::time::Instant>,
}
pub struct FetcherUpdates {
    ledger: PostgresLedger,
    client: Option<reqwest::Client>,
    cache: Mutex<Cache>,
}
fn version(value: &str) -> Option<[i32; 4]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut result = [0; 4];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 5
            || !part.bytes().all(|c| c.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        let n: i32 = part.parse().ok()?;
        if n > 65535 {
            return None;
        }
        result[i] = n;
    }
    Some(result)
}
impl FetcherUpdates {
    pub fn new(ledger: PostgresLedger) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(8))
                .build()
                .ok(),
            cache: Mutex::new(Cache::default()),
        })
    }
    pub async fn check(&self, body: &[u8], headers: &HeaderMap) -> Result<(), Response> {
        // Collect v1's platform is the producer discriminator. Other API producers stay independent.
        if !is_fetcher(body, headers) {
            return Ok(());
        }
        let policy = self.policy().await.map_err(|()| refusal(false))?;
        match headers
            .get(RUNTIME_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(version)
        {
            Some(runtime) if runtime >= policy.minimum => Ok(()),
            // One-way legacy rollout: old manifests impose no minimum. Once a positive minimum
            // is published, missing runtime identity must update to the gate build.
            None if policy.minimum == [0; 4] && !headers.contains_key(RUNTIME_HEADER) => Ok(()),
            _ => Err(refusal(true)),
        }
    }
    async fn policy(&self) -> Result<Policy, ()> {
        // Single-flight and bounded negative cache; requests cannot create an OSS storm.
        let mut cache = self.cache.lock().await;
        if cache
            .until
            .is_some_and(|until| tokio::time::Instant::now() < until)
        {
            return cache.result.clone().unwrap_or(Err(()));
        }
        let result = tokio::time::timeout(Duration::from_secs(9), self.fetch())
            .await
            .map_err(|_| ())
            .and_then(|v| v);
        cache.until = Some(
            tokio::time::Instant::now()
                + Duration::from_secs(if result.is_ok() { 300 } else { 10 }),
        );
        cache.result = Some(result.clone());
        result
    }
    async fn fetch(&self) -> Result<Policy, ()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ())?
            .as_millis();
        let mut response = self
            .client
            .as_ref()
            .ok_or(())?
            .get(format!("{ROOT}latest.json?check={now}"))
            .header(header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(|_| ())?;
        let date = response
            .headers()
            .get(header::DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
            .ok_or(())?
            .timestamp_millis();
        let age = response
            .headers()
            .get(header::AGE)
            .map(|v| v.to_str().ok().and_then(|s| s.parse::<u64>().ok()))
            .unwrap_or(Some(0))
            .ok_or(())?;
        if response.status() != StatusCode::OK
            || (now as i128 - i128::from(date)).abs() > 120000
            || age > 60
            || response.content_length().is_some_and(|n| n > 65536)
        {
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let latest = version(&m.version).ok_or(())?;
        let built = chrono::DateTime::parse_from_rfc3339(&m.built_at)
            .map_err(|_| ())?
            .timestamp_millis();
        let filename = format!("NexoFolio-Fetcher-{}-dev.zip", m.version);
        if m.schema_version != 1
            || m.product != "nexofolio-fetcher"
            || m.channel != "development"
            || m.installation != "load-unpacked"
            || version(&m.minimum_chrome_version).is_none()
            || i128::from(built) > now as i128 + 300000
            || m.artifact.filename != filename
            || m.artifact.url != format!("{ROOT}{filename}")
            || m.artifact.size == 0
            || m.artifact.size > 512 * 1024 * 1024
            || m.artifact.sha256.len() != 64
            || !m
                .artifact
                .sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(());
        }
        let (revision, minimum) = if let Some(policy_version) = m.policy_version {
            let revision = m.policy_revision.ok_or(())?;
            let minimum = version(m.minimum_version.as_deref().ok_or(())?).ok_or(())?;
            let mode = m.release_mode.as_deref().ok_or(())?;
            let notes = m.release_notes.as_ref().ok_or(())?;
            if policy_version != 1
                || !(1..=9_007_199_254_740_991).contains(&revision)
                || minimum > latest
                || !["force", "normal"].contains(&mode)
                || (mode == "force" && minimum != latest)
                || notes.len() > 32
                || notes.iter().any(|n| n.chars().count() > 1000)
            {
                return Err(());
            }
            (revision, minimum)
        } else {
            if m.policy_revision.is_some()
                || m.minimum_version.is_some()
                || m.release_mode.is_some()
                || m.release_notes.is_some()
            {
                return Err(());
            }
            (0, [0; 4])
        };
        let fingerprint = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if !self
            .ledger
            .remember_fetcher_policy(revision, &minimum, &latest, &fingerprint)
            .await
            .map_err(|_| ())?
        {
            return Err(());
        }
        Ok(Policy { minimum })
    }
}
pub(super) fn is_fetcher(body: &[u8], headers: &HeaderMap) -> bool {
    headers.contains_key(RUNTIME_HEADER)
        || serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("platform")
                    .and_then(|p| p.as_str())
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("nexofolio-fetcher")
}

pub(super) fn refusal(required: bool) -> Response {
    let (status, code, message) = if required {
        (
            StatusCode::UPGRADE_REQUIRED,
            "FETCHER_UPDATE_REQUIRED",
            "Update the running NexoFolio Fetcher extension before uploading.",
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "FETCHER_UPDATE_POLICY_UNAVAILABLE",
            "Fetcher OSS policy verification unavailable; retain and retry the unchanged batch.",
        )
    };
    (
        status,
        [
            (header::RETRY_AFTER, "10"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(serde_json::json!({"error":{"code":code,"message":message}})),
    )
        .into_response()
}
