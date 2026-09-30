//! Knowledge API: the catalogue tree, folder contents, and the curation
//! written about endpoints. Read only; every route needs login and access.

use crate::http::access::{AccessError, no_store, session_auth};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use nexofolio_access_contracts::{ProjectAccess, SessionPrincipal, Sessions};
use nexofolio_common::{FolderId, ProjectId};
use nexofolio_contracts::knowledge::{
    Author, EndpointPage, KnowledgeError, KnowledgeReader,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize)]
struct InterfacesQuery {
    directory_id: FolderId,
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Deserialize)]
struct PaginationQuery {
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    100
}

#[derive(Clone)]
struct KnowledgeHttp {
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeReader>,
    endpoints: Arc<dyn nexofolio_contracts::endpoint::EndpointReader>,
}

pub fn routes(
    sessions: Arc<dyn Sessions>,
    projects: Arc<dyn ProjectAccess>,
    knowledge: Arc<dyn KnowledgeReader>,
    endpoints: Arc<dyn nexofolio_contracts::endpoint::EndpointReader>,
) -> Router {
    Router::new()
        .route("/v1/projects/{project_id}/catalog", get(catalog))
        .route("/v1/projects/{project_id}/catalog/interfaces", get(interfaces))
        .route("/v1/projects/{project_id}/catalog/unclassified", get(unclassified))
        .route_layer(middleware::from_fn_with_state(sessions, session_auth))
        .with_state(KnowledgeHttp {
            projects,
            knowledge,
            endpoints,
        })
        .layer(middleware::from_fn(no_store))
}

enum KnowledgeHttpError {
    Access(AccessError),
    NotFound,
    Unavailable,
}

impl From<AccessError> for KnowledgeHttpError {
    fn from(error: AccessError) -> Self {
        Self::Access(error)
    }
}

impl From<KnowledgeError> for KnowledgeHttpError {
    fn from(error: KnowledgeError) -> Self {
        match error {
            KnowledgeError::NoSuchFolder => Self::NotFound,
            KnowledgeError::Unavailable => Self::Unavailable,
            _ => Self::Unavailable,
        }
    }
}

impl IntoResponse for KnowledgeHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Access(error) => return error.into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND", "目录不存在"),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_UNAVAILABLE",
                "服务暂时不可用",
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

async fn catalog(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let catalog = s.knowledge.catalogue(project).await?;
    Ok(Json(json!(catalog)))
}

async fn interfaces(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(query): Query<InterfacesQuery>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let page = s
        .knowledge
        .folder_endpoints(project, Some(query.directory_id), query.page, query.limit)
        .await?;
    Ok(Json(cards(&s, project, page).await?))
}

async fn unclassified(
    State(s): State<KnowledgeHttp>,
    Extension(principal): Extension<SessionPrincipal>,
    Path(project): Path<ProjectId>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<Value>, KnowledgeHttpError> {
    s.projects
        .require_project(&principal, project)
        .await
        .map_err(AccessError::from)?;
    let page = s
        .knowledge
        .folder_endpoints(project, None, query.page, query.limit)
        .await?;
    Ok(Json(cards(&s, project, page).await?))
}

/// An endpoint id on its own is useless to a list: the row needs the method and
/// path traffic proved, plus whatever describe has written about it. Both come
/// from one pass over the page, so a folder costs two queries however long it is.
async fn cards(
    s: &KnowledgeHttp,
    project: ProjectId,
    page: EndpointPage,
) -> Result<Value, KnowledgeHttpError> {
    let notes: HashMap<_, _> = s
        .knowledge
        .notes(project)
        .await?
        .into_iter()
        .map(|note| (note.endpoint, note))
        .collect();

    let mut items = Vec::with_capacity(page.items.len());
    for endpoint in &page.items {
        let Ok(Some(facts)) = s.endpoints.get(*endpoint).await else {
            continue;
        };
        let note = notes.get(endpoint);
        items.push(json!({
            "interface_id": endpoint,
            "method": facts.summary.method,
            "path": facts.summary.path_template,
            "name": note.and_then(|n| n.name.clone()),
            "purpose": note.and_then(|n| n.purpose.clone()),
            "described_by": note.map(|n| match &n.author {
                Author::Person { .. } => "person",
                Author::Curate { .. } => "curate",
            }),
        }));
    }

    Ok(json!({
        "items": items,
        "total": page.total,
        "page": page.page,
        "limit": page.limit,
    }))
}
