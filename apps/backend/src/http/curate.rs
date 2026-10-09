//! Curate API: trigger describe and organize passes that write Knowledge Commands.

use crate::http::access::{AccessError, no_store, session_auth};
use crate::runtime::curation::{Curation, CurationError, Progress};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{EndpointId, ProjectId};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
struct CurateHttp {
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    curation: Curation,
}

#[derive(Default, Deserialize)]
struct StartOptions {
    reuse_candidate: Option<uuid::Uuid>,
}

pub(crate) fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    curation: Curation,
) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project_id}/curate",
            get(progress).post(start),
        )
        .route(
            "/v1/projects/{project_id}/curate/describe/{endpoint_id}",
            post(describe_endpoint),
        )
        .route("/v1/projects/{project_id}/curate/organize", post(start))
        .route(
            "/v1/projects/{project_id}/curate/candidates/{candidate_id}/resume",
            post(resume),
        )
        .route(
            "/v1/projects/{project_id}/curate/candidates/{candidate_id}/cancel",
            post(cancel),
        )
        .route_layer(middleware::from_fn_with_state(
            sessions.clone(),
            session_auth,
        ))
        .with_state(CurateHttp {
            sessions,
            projects,
            curation,
        })
        .layer(middleware::from_fn(no_store))
}

enum CurateHttpError {
    Access(AccessError),
    Curation(CurationError),
}
impl From<AccessError> for CurateHttpError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}
impl From<CurationError> for CurateHttpError {
    fn from(error: CurationError) -> Self {
        Self::Curation(error)
    }
}

impl IntoResponse for CurateHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::Curation(CurationError::NotFound) => {
                (StatusCode::NOT_FOUND, "NOT_FOUND", "接口不存在".to_string())
            }
            Self::Curation(CurationError::Unavailable) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用".to_string(),
            ),
            Self::Curation(CurationError::Failed(msg)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "CURATION_FAILED", msg)
            }
            Self::Curation(CurationError::Conflict(message)) => {
                (StatusCode::CONFLICT, "CURATION_CONFLICT", message)
            }
            Self::Curation(CurationError::Busy) => (
                StatusCode::CONFLICT,
                "CURATION_RUNNING",
                "项目正在整理，请等待完成后再重新生成。".into(),
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

async fn describe_endpoint(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, endpoint)): Path<(ProjectId, EndpointId)>,
) -> Result<(StatusCode, Json<Progress>), CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(
            s.curation
                .clone()
                .with_actor(
                    super::curation_actor::from_session(s.sessions.as_ref(), &principal)
                        .await
                        .map_err(AccessError::from)?,
                )
                .start_target(project, Some(endpoint), None)
                .await?,
        ),
    ))
}

async fn start(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    options: Option<Json<StartOptions>>,
) -> Result<(StatusCode, Json<Progress>), CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;

    let reuse = options.and_then(|options| options.0.reuse_candidate);
    let curation = s.curation.clone().with_actor(
        super::curation_actor::from_session(s.sessions.as_ref(), &principal)
            .await
            .map_err(AccessError::from)?,
    );
    let progress = if reuse.is_some() {
        curation.start_target(project, None, reuse).await?
    } else {
        curation.start(project).await?
    };
    Ok((StatusCode::ACCEPTED, Json(progress)))
}

async fn progress(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Progress>, CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    Ok(Json(s.curation.saved_progress(project).await?))
}

async fn resume(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<(StatusCode, Json<Progress>), CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            s.curation
                .clone()
                .with_actor(
                    super::curation_actor::from_session(s.sessions.as_ref(), &principal)
                        .await
                        .map_err(AccessError::from)?,
                )
                .resume_candidate(project, id)
                .await?,
        ),
    ))
}
async fn cancel(
    State(s): State<CurateHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path((project, id)): Path<(ProjectId, uuid::Uuid)>,
) -> Result<StatusCode, CurateHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    s.curation.cancel(project, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
