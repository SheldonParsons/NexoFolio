use super::{Audit, Context};
use axum::{
    extract::{ConnectInfo, MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use nexofolio_audit_contracts::Event;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) fn client_ip(
    peer: Option<IpAddr>,
    forwarded: Option<&str>,
    trusted: &[IpAddr],
) -> Option<IpAddr> {
    let mut current = peer?;
    if !trusted.contains(&current) {
        return Some(current);
    }
    if let Some(header) = forwarded.filter(|s| s.len() <= 1024) {
        let chain = header
            .split(',')
            .map(|s| s.trim().parse::<IpAddr>())
            .collect::<Result<Vec<_>, _>>();
        if let Ok(chain) = chain {
            for ip in chain.into_iter().rev() {
                if !trusted.contains(&current) {
                    break;
                }
                current = ip;
            }
        }
    }
    Some(current)
}
fn resource(path: &str, segment: &str) -> String {
    let parts: Vec<_> = path.split('/').collect();
    parts
        .windows(2)
        .find(|w| w[0] == segment)
        .and_then(|w| uuid::Uuid::parse_str(w[1]).ok())
        .map(|id| id.to_string())
        .unwrap_or_default()
}
pub(crate) async fn observe(audit: Audit, mut request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path.starts_with("/health/")
        || path == "/v1/telemetry"
        || path.starts_with("/v1/audit")
        || path.starts_with("/v1/project-management")
    {
        return next.run(request).await;
    }
    let started = Instant::now();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str())
        .unwrap_or("unmatched")
        .to_owned();
    let ip = client_ip(
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip()),
        request
            .headers()
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok()),
        &audit.settings.trusted_proxies,
    );
    let action = if request.method() == "POST" && route.contains("/curate") {
        match path.rsplit('/').next().unwrap_or("") {
            "apply" => "curation.apply",
            "discard" => "curation.discard",
            "resume" => "curation.resume",
            "cancel" => "curation.cancel",
            _ => "curation.start",
        }
    } else {
        "request"
    };
    let ctx = Context {
        audit: audit.clone(),
        event: Arc::new(Mutex::new(Event {
            at: chrono::Utc::now(),
            kind: "api".into(),
            action: action.into(),
            actor: ip
                .map(|ip| format!("ip:{ip}"))
                .unwrap_or_else(|| "unknown".into()),
            route,
            method: match request.method().as_str() {
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "CONNECT"
                | "TRACE" => request.method().to_string(),
                _ => "OTHER".into(),
            },
            status: 0,
            duration_ms: 0,
            request_id: request
                .extensions()
                .get::<crate::http::RequestId>()
                .map(|i| i.0.clone())
                .unwrap_or_default(),
            project: resource(path, "projects"),
            resource: ["candidates", "endpoints", "folders"]
                .iter()
                .map(|segment| resource(path, segment))
                .find(|id| !id.is_empty())
                .unwrap_or_default(),
            code: String::new(),
        })),
    };
    request.extensions_mut().insert(ctx.clone());
    let response = next.run(request).await;
    let mut event = ctx.event();
    event.status = response.status().as_u16();
    event.duration_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    event.code = response
        .extensions()
        .get::<ProblemCode>()
        .map(|c| c.0.to_string())
        .unwrap_or_default();
    if response.status().is_server_error() || event.status == 408 {
        let mut error = event.clone();
        error.kind = "error".into();
        if error.code.is_empty() {
            error.code = format!("HTTP_{}", error.status);
        }
        tracing::error!(request_id=%error.request_id,code=%error.code,route=%error.route,"request_failed");
        audit.emit(error);
    }
    if event.action.starts_with("curation.") {
        let mut operation = event.clone();
        operation.kind = "operation".into();
        audit.emit(operation);
    }
    audit.emit(event);
    response
}
#[derive(Clone)]
pub(crate) struct ProblemCode(pub &'static str);
pub(crate) fn problem(mut response: Response, code: &'static str) -> Response {
    response.extensions_mut().insert(ProblemCode(code));
    response
}
