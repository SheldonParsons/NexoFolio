//! Fixed release selectors only; target URLs are constructed by the validated manifest parsers.
use super::{DesktopChannel, DownloadService, ReleaseState, ReleaseStatus};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::PathRejection, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoQuery {}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DesktopPlatform {
    Windows,
    MacArm64,
    MacX64,
}
impl DesktopPlatform {
    fn index(self) -> usize {
        match self {
            Self::Windows => 0,
            Self::MacArm64 => 1,
            Self::MacX64 => 2,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct DesktopTarget {
    channel: DesktopChannel,
    platform: DesktopPlatform,
}

fn no_store(mut response: Response) -> Response {
    // A permanent or cached redirect could pin a long-lived announcement to an old installer.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn error(status: StatusCode, code: &'static str) -> Response {
    no_store((status, Json(serde_json::json!({"error":code}))).into_response())
}

fn release<T>(state: ReleaseState<T>, url: impl FnOnce(T) -> String) -> Response {
    match (state.status, state.release) {
        (ReleaseStatus::Ready, Some(release)) => {
            no_store(Redirect::temporary(&url(release)).into_response())
        }
        (ReleaseStatus::Unpublished, _) => error(StatusCode::NOT_FOUND, "download_unpublished"),
        _ => error(StatusCode::SERVICE_UNAVAILABLE, "download_unavailable"),
    }
}

pub(super) async fn fetcher(
    State(service): State<Arc<DownloadService>>,
    query: Result<Query<NoQuery>, QueryRejection>,
) -> Response {
    if query.is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid_download_query");
    }
    release(service.fetcher.get().await, |release| release.artifact.url)
}

pub(super) async fn desktop(
    State(service): State<Arc<DownloadService>>,
    target: Result<Path<DesktopTarget>, PathRejection>,
    query: Result<Query<NoQuery>, QueryRejection>,
) -> Response {
    let Ok(Path(target)) = target else {
        return error(StatusCode::NOT_FOUND, "download_not_found");
    };
    if query.is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid_download_query");
    }
    let source = &service.desktop_sources(target.channel)[target.platform.index()];
    release(source.get().await, |release| release.url)
}
