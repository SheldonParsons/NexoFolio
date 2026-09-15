use super::{DesktopRelease, FetcherRelease, MAX_MANIFEST_BYTES, ReleaseStatus};
use reqwest::Url;
use serde::Deserialize;

/// Deployment-owned HTTPS roots only. Do not emit the input on configuration errors.
pub(super) fn release_directory(value: &str) -> Option<Url> {
    let mut url = Url::parse(value).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Some(url)
}

fn valid_version(version: &str) -> bool {
    version.len() <= 128 && semver::Version::parse(version).is_ok()
}
fn valid_size(size: u64) -> bool {
    size > 0 && size <= 9_007_199_254_740_991
}

/// Manifests name a single file, never an arbitrary URL or relative path.
/// Push treats the filename as a single path segment and encodes spaces.
fn artifact(directory: &Url, filename: &str, extension: &str) -> Result<String, ReleaseStatus> {
    if filename.len() > 255
        || !filename.ends_with(extension)
        || filename.len() <= extension.len()
        || filename.starts_with('.')
        || filename.trim() != filename
        || filename
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '%' | '?' | '#' | ':'))
    {
        return Err(ReleaseStatus::Error);
    }
    let mut url = directory.clone();
    url.path_segments_mut()
        .map_err(|_| ReleaseStatus::Error)?
        .pop_if_empty()
        .push(filename);
    Ok(url.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetcherManifest {
    schema_version: u32,
    product: String,
    channel: String,
    version: String,
    installation: String,
    minimum_chrome_version: String,
    artifact: FetcherArtifact,
}
#[derive(Deserialize)]
struct FetcherArtifact {
    filename: String,
    size: u64,
    sha256: String,
}

pub(super) fn parse_fetcher(
    bytes: &[u8],
    directory: &Url,
) -> Result<FetcherRelease, ReleaseStatus> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ReleaseStatus::Error);
    }
    let data: FetcherManifest = serde_json::from_slice(bytes).map_err(|_| ReleaseStatus::Error)?;
    let expected = format!("NexoFolio-Fetcher-{}-dev.zip", data.version);
    if data.schema_version != 1
        || data.product != "nexofolio-fetcher"
        || data.channel != "development"
        || data.installation != "load-unpacked"
        || !valid_version(&data.version)
        || data.artifact.filename != expected
        || !valid_size(data.artifact.size)
        || data.artifact.sha256.len() != 64
        || !data
            .artifact
            .sha256
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || data.minimum_chrome_version.len() > 32
        || !data
            .minimum_chrome_version
            .split('.')
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(ReleaseStatus::Error);
    }
    let url = artifact(directory, &expected, ".zip")?;
    Ok(FetcherRelease {
        artifact: DesktopRelease {
            version: data.version,
            url,
            filename: expected,
            size: data.artifact.size,
        },
        sha256: data.artifact.sha256,
        minimum_chrome_version: data.minimum_chrome_version,
        channel: data.channel,
        installation: data.installation,
    })
}

#[derive(Deserialize)]
struct DesktopManifest {
    version: String,
    files: Vec<DesktopFile>,
}
#[derive(Deserialize)]
struct DesktopFile {
    url: String,
    size: u64,
}

fn parse_desktop(
    bytes: &[u8],
    directory: &Url,
    extension: &str,
) -> Result<DesktopRelease, ReleaseStatus> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ReleaseStatus::Error);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ReleaseStatus::Error)?;
    let options = serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_documents: 1, max_depth: 16, flow_nesting_limit: 16,
            max_aliases: 0, max_anchors: 0, max_merge_keys: 0,
            max_nodes: 2048, max_events: 4096, max_total_scalar_bytes: MAX_MANIFEST_BYTES,
        },
        duplicate_keys: serde_saphyr::options::DuplicateKeyPolicy::Error,
        merge_keys: serde_saphyr::options::MergeKeyPolicy::Error,
        reject_unsupported_tags: true,
        with_snippet: false,
    };
    let data: DesktopManifest =
        serde_saphyr::from_str_with_options(text, options).map_err(|_| ReleaseStatus::Error)?;
    if !valid_version(&data.version) || data.files.len() > 64 {
        return Err(ReleaseStatus::Error);
    }
    let file = data
        .files
        .into_iter()
        .find(|file| file.url.ends_with(extension))
        .ok_or(ReleaseStatus::Unpublished)?;
    if !valid_size(file.size) {
        return Err(ReleaseStatus::Error);
    }
    let url = artifact(directory, &file.url, extension)?;
    Ok(DesktopRelease {
        version: data.version,
        url,
        filename: file.url,
        size: file.size,
    })
}

pub(super) fn parse_windows(
    bytes: &[u8],
    directory: &Url,
) -> Result<DesktopRelease, ReleaseStatus> {
    parse_desktop(bytes, directory, ".exe")
}
pub(super) fn parse_mac(bytes: &[u8], directory: &Url) -> Result<DesktopRelease, ReleaseStatus> {
    parse_desktop(bytes, directory, ".dmg")
}
