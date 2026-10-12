//! Privileged management endpoints: even anonymous/ordinary callers see only 404.
use super::{
    access::{AccessError, no_store},
    management::administrator,
};
use crate::runtime::project_management::ProjectAdministration;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use nexofolio_access_contracts::{SessionPrincipal, Sessions};
use nexofolio_common::{Error, ProjectId};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
struct StateData {
    service: Arc<ProjectAdministration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProject {
    name: String,
    permission_source_project_id: Option<ProjectId>,
    permission_source_project_ids: Option<Vec<ProjectId>>,
}
impl CreateProject {
    fn sources(&self) -> nexofolio_common::Result<Vec<ProjectId>> {
        match (
            &self.permission_source_project_ids,
            self.permission_source_project_id,
        ) {
            (Some(ids), None) => Ok(ids.clone()),
            (None, Some(id)) => Ok(vec![id]),
            _ => Err(Error::invalid("provide permission_source_project_ids")),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditProject {
    name: String,
    permission_source_project_ids: Vec<ProjectId>,
    revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmClear {
    confirmation: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "limit")]
    limit: u32,
}
fn first() -> u32 {
    1
}
fn limit() -> u32 {
    50
}

async fn sources(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Query(page): Query<Page>,
) -> Result<Response, AccessError> {
    Ok(Json(
        state
            .service
            .access
            .permission_sources(&principal, page.page, page.limit)
            .await?,
    )
    .into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagementQuery {
    #[serde(default = "first")]
    page: u32,
    #[serde(default = "limit")]
    limit: u32,
    #[serde(default)]
    q: String,
    origin: Option<String>,
}
async fn list(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Query(query): Query<ManagementQuery>,
) -> Result<Response, AccessError> {
    Ok(Json(
        state
            .service
            .access
            .management_projects(
                &principal,
                query.page,
                query.limit,
                &query.q,
                query.origin.as_deref(),
            )
            .await?,
    )
    .into_response())
}

async fn create(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    audit: Option<Extension<crate::audit::Context>>,
    Json(body): Json<CreateProject>,
) -> Result<Response, AccessError> {
    let project = state
        .service
        .access
        .create_local_project(&principal, &body.name, &body.sources()?)
        .await?;
    if let Some(Extension(audit)) = audit {
        audit.record(
            "operation",
            "project.created",
            "OK",
            Some(project.project_id.to_string()),
            0,
        );
    }
    Ok((StatusCode::CREATED, Json(project)).into_response())
}
async fn project(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Response, AccessError> {
    Ok(Json(
        state
            .service
            .access
            .managed_project(&principal, project)
            .await?,
    )
    .into_response())
}
async fn edit(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    audit: Option<Extension<crate::audit::Context>>,
    Json(body): Json<EditProject>,
) -> Result<Response, AccessError> {
    match state.service.access.edit_local_project(&principal, project, &body.name, &body.permission_source_project_ids, body.revision).await {
        Ok(project) => {
            if let Some(Extension(audit)) = audit { audit.record("operation", "project.updated", "OK", Some(project.project_id.to_string()), 0); }
            Ok(Json(project).into_response())
        }
        Err(Error::Conflict) => Ok((StatusCode::CONFLICT, Json(serde_json::json!({"error":{"code":"PROJECT_EDIT_CONFLICT","message":"项目已被修改、正在处理其他操作，或名称已被占用。请重新加载后再试。"}}))).into_response()),
        Err(error) => Err(error.into()),
    }
}
async fn contents(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Response, AccessError> {
    Ok(Json(state.service.contents(&principal, project, None).await?).into_response())
}
async fn clear(
    State(state): State<StateData>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    audit: Option<Extension<crate::audit::Context>>,
    Json(body): Json<ConfirmClear>,
) -> Result<Response, AccessError> {
    match state.service.contents(&principal,project,Some(&body.confirmation)).await {
        Ok(counts)=>{
            if let Some(Extension(audit))=audit { audit.record("operation","project.contents_cleared","OK",Some(project.to_string()),0); }
            Ok(Json(serde_json::json!({"cleared":counts})).into_response())
        },
        Err(Error::Conflict)=>Ok((StatusCode::CONFLICT,Json(serde_json::json!({"error":{"code":"PROJECT_BUSY","message":"项目正在采集、整理或处理其他操作，请先停止整理和采集后重试。"}}))).into_response()),
        Err(e)=>Err(e.into()),
    }
}

pub(crate) fn routes(sessions: Arc<dyn Sessions>, service: Arc<ProjectAdministration>) -> Router {
    let state = StateData { service };
    Router::new()
        .route("/v1/project-management/permission-sources", get(sources))
        .route("/v1/project-management/projects", get(list).post(create))
        .route(
            "/v1/project-management/projects/{project_id}",
            get(project).patch(edit),
        )
        .route(
            "/v1/project-management/projects/{project_id}/content",
            get(contents),
        )
        .route(
            "/v1/project-management/projects/{project_id}/clear",
            post(clear),
        )
        .route_layer(middleware::from_fn_with_state(sessions, administrator))
        .with_state(state)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
}
