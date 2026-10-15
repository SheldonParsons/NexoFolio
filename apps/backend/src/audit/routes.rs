//! Only the fixed, authenticated account can read audit data. Telemetry cannot supply identity.
use super::{Audit, http::client_ip};
use crate::http::{
    RequestId,
    access::{AccessError, no_store, session_auth},
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access_contracts::{SessionPrincipal, Sessions};
use nexofolio_audit_contracts::{AuditStore, Event, Filter};
use nexofolio_common::{Error, Secret};
use serde::Deserialize;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct StateData {
    sessions: Arc<dyn Sessions>,
    actors: Option<Arc<dyn nexofolio_access_contracts::ActorNames>>,
    store: Arc<dyn AuditStore>,
    rate: Arc<Mutex<Rate>>,
}
struct Rate {
    start: Instant,
    count: u16,
    ips: HashMap<Option<IpAddr>, u16>,
}
impl Rate {
    fn allow(&mut self, ip: Option<IpAddr>) -> bool {
        if self.start.elapsed() >= Duration::from_secs(60) {
            self.start = Instant::now();
            self.count = 0;
            self.ips.clear();
        }
        if self.count >= 1200 {
            return false;
        }
        let count = self.ips.entry(ip).or_default();
        if *count >= 120 {
            return false;
        }
        *count += 1;
        self.count += 1;
        true
    }
}
pub(crate) fn routes(
    sessions: Arc<dyn Sessions>,
    store: Arc<dyn AuditStore>,
    actors: Option<Arc<dyn nexofolio_access_contracts::ActorNames>>,
) -> Router {
    let state = StateData {
        sessions: sessions.clone(),
        actors,
        store,
        rate: Arc::new(Mutex::new(Rate {
            start: Instant::now(),
            count: 0,
            ips: HashMap::new(),
        })),
    };
    Router::new()
        .route("/v1/audit", get(report))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .route("/v1/telemetry", post(ingest))
        .with_state(state)
        .layer(DefaultBodyLimit::max(2048))
        .layer(middleware::from_fn(no_store))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportQuery {
    route: Option<String>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
    actor: Option<String>,
    request_id: Option<String>,
    project: Option<String>,
    kind: Option<String>,
}
async fn report(
    State(s): State<StateData>,
    Extension(p): Extension<SessionPrincipal>,
    audit: Option<Extension<Audit>>,
    Query(q): Query<ReportQuery>,
) -> Result<Json<serde_json::Value>, AccessError> {
    let profile = s.sessions.me(&p).await?;
    if profile.id != p.user_id
        || !profile.enabled
        || !profile.account.eq_ignore_ascii_case("a80646")
    {
        return Err(Error::Forbidden.into());
    }
    if [&q.actor, &q.request_id, &q.project, &q.kind]
        .iter()
        .any(|v| v.as_ref().is_some_and(|s| s.len() > 64))
    {
        return Err(Error::invalid("invalid audit filter").into());
    }
    if q.route.as_ref().is_some_and(|r| r.len() > 200) {
        return Err(Error::invalid("invalid route filter").into());
    }
    let since = q
        .since
        .unwrap_or_else(|| chrono::Utc::now() - chrono::TimeDelta::days(7));
    let until = q.until.unwrap_or_else(chrono::Utc::now);
    if since > until || until - since > chrono::TimeDelta::days(366) {
        return Err(Error::invalid("invalid audit range").into());
    }
    let mut data = s
        .store
        .report(&Filter {
            route: q.route,
            since: Some(since),
            until: Some(until),
            actor: q.actor,
            request_id: q.request_id,
            project: q.project,
            kind: q.kind,
            limit: 200,
        })
        .await?;
    if let Some(actors) = &s.actors {
        let ids: std::collections::HashSet<nexofolio_common::UserId> = ["events", "buckets"]
            .iter()
            .filter_map(|key| data[*key].as_array())
            .flatten()
            .filter_map(|row| row["actor"].as_str())
            .filter_map(|value| value.strip_prefix("user:"))
            .filter_map(|value| value.parse().ok())
            .collect();
        if let Ok(names) = actors
            .actor_names(&ids.into_iter().collect::<Vec<_>>())
            .await
        {
            data["actors"] = serde_json::Value::Object(
                names
                    .into_iter()
                    .map(|name| {
                        (
                            format!("user:{}", name.id),
                            serde_json::Value::String(name.display_name),
                        )
                    })
                    .collect(),
            );
        }
    }
    if let Some(Extension(audit)) = audit {
        data["health"] = audit.health();
        data["limits"] = serde_json::json!({"events":audit.settings.limits.events,"buckets":audit.settings.limits.buckets,"access_days":audit.settings.limits.access_days,"error_days":audit.settings.limits.error_days,"operation_days":audit.settings.limits.operation_days});
    }
    Ok(Json(data))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Telemetry {
    kind: String,
    route: String,
    project: Option<uuid::Uuid>,
    resource: Option<uuid::Uuid>,
    code: Option<String>,
    request_id: Option<uuid::Uuid>,
}
fn page(route: &str) -> bool {
    matches!(
        route,
        "home"
            | "projects"
            | "project-interfaces"
            | "document-folder"
            | "document-endpoint"
            | "curation-review"
            | "product-docs"
            | "product-docs:philosophy"
            | "product-docs:fetcher"
            | "product-docs:mcp"
            | "product-docs:api"
            | "product-docs:retirement"
            | "product-docs:unknown"
            | "product-changelog"
            | "settings"
            | "mcp-credentials"
            | "not-found"
    )
}
async fn ingest(
    State(s): State<StateData>,
    audit: Option<Extension<Audit>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    Json(body): Json<Telemetry>,
) -> Response {
    let Some(Extension(audit)) = audit else {
        return StatusCode::NO_CONTENT.into_response();
    };
    if !audit.settings.enabled {
        return StatusCode::NO_CONTENT.into_response();
    }
    if !page(&body.route)
        || !matches!(body.kind.as_str(), "page" | "error")
        || body.code.as_ref().is_some_and(|s| {
            !matches!(
                s.as_str(),
                "JS_ERROR"
                    | "UNHANDLED_REJECTION"
                    | "VUE_ERROR"
                    | "ROUTE_ERROR"
                    | "NETWORK_ERROR"
                    | "CLIENT_TIMEOUT"
                    | "INVALID_RESPONSE"
            )
        })
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let ip = client_ip(
        peer.map(|Extension(c)| c.0.ip()),
        headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()),
        &audit.settings.trusted_proxies,
    );
    if !s.rate.lock().unwrap_or_else(|e| e.into_inner()).allow(ip) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let mut actor = ip
        .map(|ip| format!("ip:{ip}"))
        .unwrap_or_else(|| "unknown".into());
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty() && v.len() <= 256);
    if let Some(token) = token
        && let Ok(Ok(p)) = tokio::time::timeout(
            Duration::from_millis(250),
            s.sessions.verify_session(&Secret::new(token)),
        )
        .await
    {
        actor = format!("user:{}", p.user_id);
    }
    audit.emit(Event {
        execution: None,
        at: chrono::Utc::now(),
        kind: body.kind.clone(),
        action: if body.kind == "page" {
            "page.view"
        } else {
            "client.error"
        }
        .into(),
        actor,
        route: body.route,
        method: "BROWSER".into(),
        status: 0,
        duration_ms: 0,
        request_id: body
            .request_id
            .map(|i| i.to_string())
            .or_else(|| id.map(|Extension(i)| i.0))
            .unwrap_or_default(),
        project: body.project.map(|i| i.to_string()).unwrap_or_default(),
        resource: body.resource.map(|i| i.to_string()).unwrap_or_default(),
        code: body.code.unwrap_or_default(),
    });
    StatusCode::NO_CONTENT.into_response()
}
