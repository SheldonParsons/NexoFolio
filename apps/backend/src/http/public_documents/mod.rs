//! Global public web reads; no member grants, samples, candidates or MCP bypass.
mod projection;
#[cfg(test)]
mod tests;
mod types;

use crate::http::access::{AccessError, no_store};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    middleware,
    routing::get,
};
use nexofolio_access_contracts::{DocumentProject, ProjectDocuments};
use nexofolio_common::{EndpointId, EnvironmentId, Error, ProjectId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::KnowledgeReader};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
struct PublicDocuments {
    instance: String,
    projects: Arc<dyn ProjectDocuments>,
    endpoints: Arc<dyn EndpointReader>,
    knowledge: Arc<dyn KnowledgeReader>,
}
pub fn routes(
    instance: String,
    projects: Arc<dyn ProjectDocuments>,
    endpoints: Arc<dyn EndpointReader>,
    knowledge: Arc<dyn KnowledgeReader>,
) -> Router {
    Router::new()
        .route("/v1/public/projects/{project_id}/document", get(index))
        .route(
            "/v1/public/projects/{project_id}/endpoints/{endpoint_id}",
            get(endpoint),
        )
        .with_state(PublicDocuments {
            instance,
            projects,
            endpoints,
            knowledge,
        })
        .layer(middleware::from_fn(no_store))
}
impl PublicDocuments {
    async fn project(&self, id: ProjectId) -> Result<DocumentProject, AccessError> {
        self.projects
            .document_project(&self.instance, id)
            .await?
            .ok_or(Error::NotFound.into())
    }
}
fn unavailable(_: impl std::fmt::Debug) -> AccessError {
    Error::Unavailable {
        component: "public_documents",
    }
    .into()
}
async fn index(
    State(s): State<PublicDocuments>,
    Path(project): Path<ProjectId>,
) -> Result<Json<types::DocumentIndex>, AccessError> {
    let metadata = s.project(project).await?;
    let (navigation, notes, summaries) = tokio::try_join!(
        async {
            s.knowledge
                .catalogue_navigation(project)
                .await
                .map_err(unavailable)
        },
        async { s.knowledge.notes(project).await.map_err(unavailable) },
        async { s.endpoints.list(project).await.map_err(unavailable) },
    )?;
    let notes: HashMap<_, _> = notes.iter().map(|n| (n.endpoint, n)).collect();
    let placements: HashMap<_, _> = navigation
        .placements
        .iter()
        .map(|p| (p.endpoint, p.folder))
        .collect();
    let entries = summaries
        .into_iter()
        .filter(|e| e.project_id == project)
        .map(|e| {
            let id = e.id;
            projection::entry(e, notes.get(&id).copied(), placements.get(&id).copied())
        })
        .collect();
    Ok(Json(types::DocumentIndex {
        lifecycle: s.knowledge.lifecycle(project).await.map_err(unavailable)?,
        project: metadata,
        catalog: projection::catalogue(navigation.catalogue),
        entries,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    environment_id: Option<EnvironmentId>,
}
async fn endpoint(
    State(s): State<PublicDocuments>,
    Path((project, endpoint)): Path<(ProjectId, EndpointId)>,
    Query(scope): Query<Scope>,
) -> Result<Json<types::DocumentEndpoint>, AccessError> {
    let metadata = s.project(project).await?;
    if scope
        .environment_id
        .is_some_and(|env| !metadata.environments.iter().any(|e| e.id == env))
    {
        return Err(Error::NotFound.into());
    }
    let facts = s
        .endpoints
        .get(endpoint)
        .await
        .map_err(unavailable)?
        .ok_or(Error::NotFound)?;
    if facts.summary.project_id != project {
        return Err(Error::NotFound.into());
    }
    let knowledge = s
        .knowledge
        .endpoint(facts.summary.id)
        .await
        .map_err(unavailable)?;
    let lifecycle = s.knowledge.lifecycle(project).await.map_err(unavailable)?;
    let retired_source_count = lifecycle.retired_source_count(&knowledge);
    let retired_fields: std::collections::HashSet<_> = knowledge
        .fields
        .iter()
        .filter(|f| lifecycle.field_sources_retired(f))
        .map(|f| f.path.clone())
        .collect();
    let mut document = projection::endpoint(facts, knowledge, scope.environment_id);
    document.retired_source_count = retired_source_count;
    for field in &mut document.fields {
        field.retired_sources = retired_fields.contains(&field.key);
    }
    Ok(Json(document))
}
