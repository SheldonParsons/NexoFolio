use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access::{LoginCredentials, PlatformAccess, SessionPrincipal};
use nexofolio_application::LoginService;
use nexofolio_contracts::{Error, ProjectId, Secret};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AccessHttp {
    pub login: Arc<LoginService>,
    pub store: Arc<dyn PlatformAccess>,
    permits: Arc<Semaphore>,
}
impl AccessHttp {
    pub fn new(login: Arc<LoginService>, store: Arc<dyn PlatformAccess>) -> Self {
        Self {
            login,
            store,
            permits: Arc::new(Semaphore::new(4)),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginBody {
    account: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pagination {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "size")]
    limit: u32,
}
fn first() -> u32 {
    1
}
fn size() -> u32 {
    20
}

pub struct AccessError(pub Error);
impl From<Error> for AccessError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for AccessError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.0 {
            Error::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHENTICATED",
                "登录凭证无效或已过期",
            ),
            Error::Forbidden => (
                StatusCode::FORBIDDEN,
                "PROJECT_ACCESS_DENIED",
                "你暂无该项目的访问权限，请联系项目负责人或禅道管理员",
            ),
            Error::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "项目不存在"),
            Error::Unavailable {
                component: "project_access",
            } => (
                StatusCode::SERVICE_UNAVAILABLE,
                "PROJECT_ACCESS_UNAVAILABLE",
                "暂时无法确认项目权限",
            ),
            Error::InvalidInput { .. } => (
                StatusCode::BAD_REQUEST,
                "VALIDATION_ERROR",
                "请求参数不符合要求",
            ),
            Error::NotConfigured { .. } => (
                StatusCode::SERVICE_UNAVAILABLE,
                "NOT_CONFIGURED",
                "登录服务尚未配置",
            ),
            Error::Conflict => (StatusCode::CONFLICT, "CONFLICT", "当前数据版本发生变化"),
            _ => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用",
            ),
        };
        let mut response = (
            status,
            Json(serde_json::json!({"error":{"code":code,"message":message}})),
        )
            .into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Bearer realm=\"nexofolio\"".parse().unwrap(),
            );
        }
        response
    }
}

pub fn routes(state: AccessHttp) -> Router {
    let protected = Router::new()
        .route("/v1/auth/me", get(me))
        .route("/v1/projects", get(projects))
        .route("/v1/projects/{project_id}", get(project))
        .route_layer(middleware::from_fn_with_state(
            state.store.clone(),
            session_auth,
        ));
    Router::new()
        .route("/v1/auth/login", post(login))
        .merge(protected)
        .with_state(state)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
}
async fn no_store(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    r.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}
async fn login(
    State(s): State<AccessHttp>,
    Json(body): Json<LoginBody>,
) -> Result<Response, AccessError> {
    let password = Secret::new(body.password);
    let account = body.account.trim().to_owned();
    if account.is_empty()
        || account.len() > 128
        || password.expose().is_empty()
        || password.expose().len() > 1024
    {
        return Err(Error::InvalidInput {
            message: "invalid credentials shape".into(),
        }
        .into());
    }
    let Ok(_permit) = s.permits.clone().try_acquire_owned() else {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "2")],
            Json(serde_json::json!({"error":{"code":"LOGIN_BUSY"}})),
        )
            .into_response());
    };
    let result = s
        .login
        .login(LoginCredentials { account, password })
        .await?;
    Ok(Json(serde_json::json!({"user":result.session.user,"token":result.session.token.expose(),"token_type":"Bearer","expires_at":result.session.expires_at,"token_reused":result.session.reused,"project_sync":result.sync})).into_response())
}
async fn session_auth(
    State(store): State<Arc<dyn PlatformAccess>>,
    mut req: Request,
    next: Next,
) -> Result<Response, AccessError> {
    let mut headers = req.headers().get_all(header::AUTHORIZATION).iter();
    let value = headers
        .next()
        .and_then(|x| x.to_str().ok())
        .ok_or(Error::Unauthenticated)?;
    if headers.next().is_some() {
        return Err(Error::Unauthenticated.into());
    }
    let (scheme, value) = value.split_once(' ').ok_or(Error::Unauthenticated)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || value.len() > 256
        || value.bytes().any(|b| b.is_ascii_whitespace())
    {
        return Err(Error::Unauthenticated.into());
    }
    let principal = store.verify_session(&Secret::new(value)).await?;
    req.extensions_mut().insert(principal);
    Ok(next.run(req).await)
}
async fn me(
    State(s): State<AccessHttp>,
    Extension(p): Extension<SessionPrincipal>,
) -> Result<Json<serde_json::Value>, AccessError> {
    Ok(Json(serde_json::json!({"user":s.store.me(&p).await?})))
}
async fn projects(
    State(s): State<AccessHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Query(q): Query<Pagination>,
) -> Result<Json<nexofolio_access::ProjectPage>, AccessError> {
    Ok(Json(s.store.list_projects(&p, q.page, q.limit).await?))
}
async fn project(
    State(s): State<AccessHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Path(id): Path<ProjectId>,
) -> Result<Json<nexofolio_access::ProjectCard>, AccessError> {
    Ok(Json(s.store.require_project(&p, id).await?))
}
