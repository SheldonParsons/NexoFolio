//! Authenticated self-profile only. No caller-supplied user identity or permissions.
use super::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use nexofolio_access_contracts::{
    PersonalProfile, PersonalProfiles, ProfileUpdate, SessionPrincipal, Sessions,
};
use nexofolio_common::Error;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
struct ProfileHttp {
    profiles: Arc<dyn PersonalProfiles>,
}
pub(crate) fn routes(sessions: Arc<dyn Sessions>, profiles: Arc<dyn PersonalProfiles>) -> Router {
    Router::new()
        .route("/v1/auth/profile", get(read).put(update))
        .route("/v1/auth/profile/avatar", post(avatar))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(ProfileHttp { profiles })
        .layer(DefaultBodyLimit::max(384 * 1024))
        .layer(middleware::from_fn(no_store))
}
struct ProfileError(Error);
impl From<Error> for ProfileError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for ProfileError {
    fn into_response(self) -> Response {
        match self.0 {
            Error::InvalidInput {message} => (StatusCode::BAD_REQUEST,Json(json!({"error":{"code":"VALIDATION_ERROR","message":message}}))).into_response(),
            Error::Conflict => (StatusCode::CONFLICT,Json(json!({"error":{"code":"PROFILE_CHANGED","message":"资料已在另一处更新，请重新读取后保存；你的输入仍保留"}}))).into_response(),
            e => AccessError(e).into_response(),
        }
    }
}
async fn read(
    State(s): State<ProfileHttp>,
    Extension(p): Extension<SessionPrincipal>,
) -> Result<Json<PersonalProfile>, ProfileError> {
    Ok(Json(s.profiles.personal_profile(&p).await?))
}
async fn update(
    State(s): State<ProfileHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Json(body): Json<ProfileUpdate>,
) -> Result<Json<PersonalProfile>, ProfileError> {
    Ok(Json(s.profiles.update_profile(&p, body).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AvatarBody {
    version: i64,
    data_url: Option<String>,
}
async fn avatar(
    State(s): State<ProfileHttp>,
    Extension(p): Extension<SessionPrincipal>,
    Json(body): Json<AvatarBody>,
) -> Result<Json<PersonalProfile>, ProfileError> {
    let bytes = body
        .data_url
        .map(|url| -> Result<Vec<u8>, Error> {
            let encoded = url
                .strip_prefix("data:image/webp;base64,")
                .ok_or_else(|| Error::invalid("请选择 JPG、PNG 或 WebP 图片并完成裁剪"))?;
            STANDARD
                .decode(encoded)
                .map_err(|_| Error::invalid("头像内容无法读取"))
        })
        .transpose()?;
    Ok(Json(
        s.profiles.update_avatar(&p, body.version, bytes).await?,
    ))
}
