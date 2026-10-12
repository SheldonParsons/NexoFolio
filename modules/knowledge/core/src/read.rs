//! Consistent read projections over knowledge-owned snapshots.
use super::{Clock, Knowledge, store_error, tree};
use async_trait::async_trait;
use nexofolio_common::{EndpointId, FolderId, ProjectId};
use nexofolio_contracts::{
    endpoint::EndpointReader,
    knowledge::{
        Catalogue, CatalogueNavigation, EndpointKnowledge, EndpointNote, EndpointPage, FieldNote,
        Folder, KnowledgeError, KnowledgeReader, KnowledgeResult, Link, MAX_DEPTH, Placement,
        Round,
    },
};
use nexofolio_knowledge_contracts::{FolderRow, KnowledgeStore};
use std::collections::{HashMap, HashSet};
#[async_trait]
impl<S, R, C> KnowledgeReader for Knowledge<S, R, C>
where
    S: KnowledgeStore,
    R: EndpointReader,
    C: Clock,
{
    async fn lifecycle(
        &self,
        project: ProjectId,
    ) -> KnowledgeResult<nexofolio_contracts::knowledge::DocumentLifecycle> {
        let state = self
            .store
            .begin_read()
            .await
            .map_err(store_error)?
            .lifecycle(project)
            .await
            .map_err(store_error)?;
        crate::lifecycle::resolve(&self.endpoints, state).await
    }
    async fn observed_values(
        &self,
        query: nexofolio_contracts::field_values::FieldValueQuery,
    ) -> KnowledgeResult<nexofolio_contracts::field_values::ObservedValuePage> {
        self.read_value_page(query).await
    }
    async fn observed_value(
        &self,
        query: nexofolio_contracts::field_values::FieldValueQuery,
        value_id: &str,
    ) -> KnowledgeResult<serde_json::Value> {
        self.read_value(query, value_id).await
    }
    async fn catalogue(&self, project: ProjectId) -> KnowledgeResult<Catalogue> {
        Ok(self.catalogue_navigation(project).await?.catalogue)
    }

    async fn catalogue_navigation(
        &self,
        project: ProjectId,
    ) -> KnowledgeResult<CatalogueNavigation> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let folders = tx.folders(project).await.map_err(store_error)?;
        let placements = tx.placements(project).await.map_err(store_error)?;

        let mut placed: HashMap<FolderId, usize> = HashMap::new();
        for p in &placements {
            *placed.entry(p.folder).or_default() += 1;
        }

        // 待分类 is a subtraction, not a row: an endpoint is browsable the
        // moment observe accepts it, with no write here to go wrong.
        let existing = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?;
        let seated: HashSet<EndpointId> = placements.iter().map(|p| p.endpoint).collect();
        let unplaced = existing.iter().filter(|e| !seated.contains(&e.id)).count();

        Ok(CatalogueNavigation {
            catalogue: Catalogue {
                project_id: project,
                folders: tree::sorted(&folders, &placed),
                unplaced,
            },
            placements: placements
                .into_iter()
                .map(|p| Placement {
                    endpoint: p.endpoint,
                    folder: p.folder,
                    author: p.author,
                    at: p.at,
                })
                .collect(),
        })
    }

    async fn folder_endpoints(
        &self,
        project: ProjectId,
        folder: Option<FolderId>,
        page: usize,
        limit: usize,
    ) -> KnowledgeResult<EndpointPage> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let placements = tx.placements(project).await.map_err(store_error)?;
        let all: Vec<EndpointId> = match folder {
            Some(folder) => {
                if tx
                    .folder(folder)
                    .await
                    .map_err(store_error)?
                    .is_none_or(|f| f.project_id != project)
                {
                    return Err(KnowledgeError::NoSuchFolder);
                }
                placements
                    .iter()
                    .filter(|p| p.folder == folder)
                    .map(|p| p.endpoint)
                    .collect()
            }
            None => {
                let seated: HashSet<EndpointId> = placements.iter().map(|p| p.endpoint).collect();
                let existing = self
                    .endpoints
                    .list(project)
                    .await
                    .map_err(|_| KnowledgeError::Unavailable)?;
                existing
                    .into_iter()
                    .map(|e| e.id)
                    .filter(|id| !seated.contains(id))
                    .collect()
            }
        };
        let total = all.len();
        let page = page.max(1);
        let limit = limit.max(1);
        let start = (page - 1) * limit;
        let items = if start < total {
            all.into_iter().skip(start).take(limit).collect()
        } else {
            Vec::new()
        };
        Ok(EndpointPage {
            items,
            total,
            page,
            limit,
        })
    }

    async fn endpoint(&self, endpoint: EndpointId) -> KnowledgeResult<EndpointKnowledge> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let note = tx.endpoint_note(endpoint).await.map_err(store_error)?;
        let placement = tx.placement(endpoint).await.map_err(store_error)?;
        let fields = tx.field_notes(endpoint).await.map_err(store_error)?;
        let links = tx.links_of(endpoint).await.map_err(store_error)?;

        // Breadcrumb: ancestors of the placement, top level first.
        let mut path = Vec::new();
        if let Some(seat) = &placement {
            let folders = tx.folders(seat.project_id).await.map_err(store_error)?;
            let by_id: HashMap<FolderId, &FolderRow> = folders.iter().map(|f| (f.id, f)).collect();
            let mut at = Some(seat.folder);
            let mut guard = 0;
            while let Some(id) = at {
                let Some(row) = by_id.get(&id) else { break };
                path.push(Folder {
                    id: row.id,
                    author: row.author.clone(),
                    parent: row.parent,
                    name: row.name.clone(),
                    summary: row.summary.clone(),
                    position: row.position,
                    endpoints: 0,
                    endpoints_deep: 0,
                });
                at = row.parent;
                guard += 1;
                if guard > MAX_DEPTH {
                    break;
                }
            }
            path.reverse();
        }

        Ok(EndpointKnowledge {
            note,
            placement: placement.map(|p| Placement {
                endpoint: p.endpoint,
                folder: p.folder,
                author: p.author,
                at: p.at,
            }),
            path,
            fields,
            links,
        })
    }

    async fn field_notes(&self, project: ProjectId) -> KnowledgeResult<Vec<FieldNote>> {
        self.store
            .begin_read()
            .await
            .map_err(store_error)?
            .project_field_notes(project)
            .await
            .map_err(store_error)
    }

    async fn notes(&self, project: ProjectId) -> KnowledgeResult<Vec<EndpointNote>> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        tx.endpoint_notes(project).await.map_err(store_error)
    }

    async fn links(&self, project: ProjectId) -> KnowledgeResult<Vec<Link>> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        tx.links(project).await.map_err(store_error)
    }

    async fn rounds(&self, project: ProjectId, limit: usize) -> KnowledgeResult<Vec<Round>> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        tx.rounds(project, limit).await.map_err(store_error)
    }
}
