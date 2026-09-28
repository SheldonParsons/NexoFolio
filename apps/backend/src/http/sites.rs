//! The server-side site registry: which project and environment a page
//! belongs to. Lookup is public so the plugin can show it before login.

use crate::http::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{EnvironmentId, ProjectId};
use nexofolio_contracts::scope::{ScopeError, SiteBinding, SiteMatch, SiteRegistry, SiteScope};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_URL_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct SitesHttp {
    projects: Arc<dyn ProjectAccess>,
    registry: Arc<dyn SiteRegistry>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    registry: Arc<dyn SiteRegistry>,
) -> Router {
    let protected = Router::new()
        .route("/v1/sites", put(bind))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth));
    Router::new()
        .route("/v1/sites/lookup", get(lookup))
        .merge(protected)
        .with_state(SitesHttp { projects, registry })
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupQuery {
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindBody {
    site: SiteScope,
    project_id: ProjectId,
    environment_id: EnvironmentId,
}

enum SitesError {
    Access(AccessError),
    Scope(ScopeError),
}

impl From<AccessError> for SitesError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<ScopeError> for SitesError {
    fn from(error: ScopeError) -> Self {
        Self::Scope(error)
    }
}

impl IntoResponse for SitesError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::Scope(ScopeError::UnknownProject) => {
                (StatusCode::NOT_FOUND, "UNKNOWN_PROJECT", "项目不存在")
            }
            Self::Scope(ScopeError::UnknownEnvironment) => (
                StatusCode::NOT_FOUND,
                "UNKNOWN_ENVIRONMENT",
                "环境不存在或不属于该项目",
            ),
            Self::Scope(ScopeError::InvalidSite(_) | ScopeError::InvalidEnvironmentName(_)) => (
                StatusCode::BAD_REQUEST,
                "VALIDATION_ERROR",
                "请求参数不符合要求",
            ),
            Self::Scope(ScopeError::Unavailable) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用",
            ),
        };
        (
            status,
            Json(json!({"error": {"code": code, "message": message}})),
        )
            .into_response()
    }
}

/// The binding with the longest prefix containing `url`, or `null`.
async fn lookup(
    State(s): State<SitesHttp>,
    Query(query): Query<LookupQuery>,
) -> Result<Json<Value>, SitesError> {
    if query.url.len() > MAX_URL_BYTES {
        return Err(ScopeError::InvalidSite("url too long".into()).into());
    }
    let found = s.registry.lookup(&query.url).await?;
    Ok(Json(found.map_or(Value::Null, |m| describe(&m))))
}

fn describe(found: &SiteMatch) -> Value {
    let binding = &found.binding;
    json!({
        "site": binding.site,
        "project": {"id": binding.project_id, "name": found.project_name},
        "environment": {"id": binding.environment_id, "name": found.environment_name},
    })
}

/// Creates or moves a binding. The caller must be able to open the project.
async fn bind(
    State(s): State<SitesHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Json(body): Json<BindBody>,
) -> Result<StatusCode, SitesError> {
    s.projects
        .require_project(&principal, body.project_id)
        .await
        .map_err(AccessError::from)?;
    s.registry
        .bind(SiteBinding {
            site: body.site,
            project_id: body.project_id,
            environment_id: body.environment_id,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
