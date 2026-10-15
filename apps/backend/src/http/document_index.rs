//! Shared, lightweight document index. Callers authorize before entering this read.
use super::access::AccessError;
use nexofolio_common::{EndpointId, Error, FolderId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub(crate) struct DocumentIndex {
    pub lifecycle: DocumentLifecycle,
    pub catalog: CatalogueIndex,
    pub entries: Vec<Entry>,
}
#[derive(Serialize)]
pub(crate) struct CatalogueIndex {
    pub project_id: ProjectId,
    pub folders: Vec<Folder>,
    pub unplaced: usize,
}
#[derive(Serialize)]
pub(crate) struct Folder {
    pub id: FolderId,
    pub parent: Option<FolderId>,
    pub name: String,
    pub summary: Option<String>,
    pub position: i32,
    pub endpoints: usize,
    pub endpoints_deep: usize,
}
#[derive(Serialize)]
pub(crate) struct Summary {
    pub id: EndpointId,
    pub project_id: ProjectId,
    pub method: String,
    pub path_template: String,
    pub declared: bool,
    pub external: bool,
    pub environments: Vec<EnvironmentUsage>,
}
#[derive(Serialize)]
pub(crate) struct Entry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub described_by: Option<&'static str>,
    pub interface_id: EndpointId,
    pub method: String,
    pub path: String,
    pub name: Option<String>,
    pub purpose: Option<String>,
    #[serde(rename = "folderId")]
    pub folder_id: Option<FolderId>,
    pub summary: Summary,
}
pub(crate) async fn read(
    project: ProjectId,
    knowledge: &dyn KnowledgeReader,
    endpoints: &dyn EndpointReader,
) -> Result<DocumentIndex, AccessError> {
    let (navigation, notes, summaries) = tokio::try_join!(
        async {
            knowledge
                .catalogue_navigation(project)
                .await
                .map_err(unavailable)
        },
        async { knowledge.notes(project).await.map_err(unavailable) },
        async { endpoints.list(project).await.map_err(unavailable) },
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
            entry(e, notes.get(&id).copied(), placements.get(&id).copied())
        })
        .collect();
    Ok(DocumentIndex {
        lifecycle: knowledge.lifecycle(project).await.map_err(unavailable)?,
        catalog: catalogue(navigation.catalogue),
        entries,
    })
}
fn unavailable(_: impl std::fmt::Debug) -> AccessError {
    Error::Unavailable {
        component: "document_index",
    }
    .into()
}
fn catalogue(c: Catalogue) -> CatalogueIndex {
    CatalogueIndex {
        project_id: c.project_id,
        unplaced: c.unplaced,
        folders: c
            .folders
            .into_iter()
            .map(|f| Folder {
                id: f.id,
                parent: f.parent,
                name: f.name,
                summary: f.summary,
                position: f.position,
                endpoints: f.endpoints,
                endpoints_deep: f.endpoints_deep,
            })
            .collect(),
    }
}
fn entry(
    s: EndpointSummary,
    note: Option<&EndpointNote>,
    folder_id: Option<nexofolio_common::FolderId>,
) -> Entry {
    Entry {
        described_by: note.map(|n| match n.author {
            Author::Person { .. } => "person",
            Author::Curate { .. } => "curate",
        }),
        interface_id: s.id,
        method: s.method.clone(),
        path: s.path_template.clone(),
        name: note.and_then(|n| n.name.clone()),
        purpose: note.and_then(|n| n.purpose.clone()),
        folder_id,
        summary: Summary {
            id: s.id,
            project_id: s.project_id,
            method: s.method,
            path_template: s.path_template,
            declared: s.declared,
            external: s.external,
            environments: s.environments,
        },
    }
}
