//! A proposal transaction reuses the official command interpreter and tree rules.
//! It cannot commit to a database; only the resulting snapshot is persisted as a draft.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge_contracts::{
    FolderRow, KnowledgeTx, LinkIdentity, PlacementRow, StoreError, StoreResult,
};
use serde_json::Value;

pub(super) struct SnapshotTx {
    pub project: ProjectId,
    pub view: KnowledgeSnapshot,
}
fn replace<T>(items: &mut Vec<T>, item: T, matches: impl Fn(&T) -> bool) {
    items.retain(|old| !matches(old));
    items.push(item);
}
pub(super) fn row(project: ProjectId, f: &Folder) -> FolderRow {
    FolderRow {
        id: f.id,
        project_id: project,
        author: f.author.clone(),
        parent: f.parent,
        name: f.name.clone(),
        summary: f.summary.clone(),
        position: f.position,
    }
}

#[async_trait]
impl KnowledgeTx for SnapshotTx {
    async fn active_candidate(&mut self, _: ProjectId) -> StoreResult<Option<CandidateSummary>> {
        Ok(None)
    }
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }
    async fn lock_project(&mut self, _: ProjectId) -> StoreResult<()> {
        Ok(())
    }
    async fn create_round(&mut self, _: &Round) -> StoreResult<()> {
        Ok(())
    }
    async fn round(&mut self, _: RoundId) -> StoreResult<Option<Round>> {
        Ok(None)
    }
    async fn finish_round(
        &mut self,
        _: RoundId,
        _: RoundState,
        _: Option<&str>,
        _: DateTime<Utc>,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn rounds(&mut self, _: ProjectId, _: usize) -> StoreResult<Vec<Round>> {
        Ok(vec![])
    }
    async fn record_revision(&mut self, _: &Revision) -> StoreResult<()> {
        Ok(())
    }
    async fn revisions(&mut self, _: RoundId) -> StoreResult<Vec<Revision>> {
        Ok(vec![])
    }
    async fn folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>> {
        Ok(self
            .view
            .folders
            .iter()
            .find(|f| f.id == id)
            .map(|f| row(self.project, f)))
    }
    async fn folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>> {
        Ok(if project == self.project {
            self.view.folders.iter().map(|f| row(project, f)).collect()
        } else {
            vec![]
        })
    }
    async fn put_folder(&mut self, f: &FolderRow) -> StoreResult<()> {
        replace(
            &mut self.view.folders,
            Folder {
                id: f.id,
                author: f.author.clone(),
                parent: f.parent,
                name: f.name.clone(),
                summary: f.summary.clone(),
                position: f.position,
                endpoints: 0,
                endpoints_deep: 0,
            },
            |old| old.id == f.id,
        );
        Ok(())
    }
    async fn remove_folder(&mut self, id: FolderId) -> StoreResult<()> {
        let mut removed = vec![id];
        let mut index = 0;
        while index < removed.len() {
            let parent = removed[index];
            removed.extend(
                self.view
                    .folders
                    .iter()
                    .filter(|f| f.parent == Some(parent))
                    .map(|f| f.id),
            );
            index += 1;
        }
        self.view.folders.retain(|f| !removed.contains(&f.id));
        self.view
            .placements
            .retain(|p| !removed.contains(&p.folder));
        Ok(())
    }
    async fn placement(&mut self, endpoint: EndpointId) -> StoreResult<Option<PlacementRow>> {
        Ok(self
            .view
            .placements
            .iter()
            .find(|p| p.endpoint == endpoint)
            .map(|p| PlacementRow {
                endpoint: p.endpoint,
                project_id: self.project,
                folder: p.folder,
                author: p.author.clone(),
                at: p.at,
            }))
    }
    async fn placements(&mut self, _: ProjectId) -> StoreResult<Vec<PlacementRow>> {
        Ok(self
            .view
            .placements
            .iter()
            .map(|p| PlacementRow {
                endpoint: p.endpoint,
                project_id: self.project,
                folder: p.folder,
                author: p.author.clone(),
                at: p.at,
            })
            .collect())
    }
    async fn put_placement(&mut self, p: &PlacementRow) -> StoreResult<()> {
        replace(
            &mut self.view.placements,
            Placement {
                endpoint: p.endpoint,
                folder: p.folder,
                author: p.author.clone(),
                at: p.at,
            },
            |old| old.endpoint == p.endpoint,
        );
        Ok(())
    }
    async fn remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()> {
        self.view.placements.retain(|p| p.endpoint != endpoint);
        Ok(())
    }
    async fn endpoint_note(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointNote>> {
        Ok(self
            .view
            .notes
            .iter()
            .find(|n| n.endpoint == endpoint)
            .cloned())
    }
    async fn endpoint_notes(&mut self, _: ProjectId) -> StoreResult<Vec<EndpointNote>> {
        Ok(self.view.notes.clone())
    }
    async fn put_endpoint_note(&mut self, _: ProjectId, note: &EndpointNote) -> StoreResult<()> {
        replace(&mut self.view.notes, note.clone(), |old| {
            old.endpoint == note.endpoint
        });
        Ok(())
    }
    async fn field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>> {
        Ok(self
            .view
            .fields
            .iter()
            .find(|n| n.endpoint == endpoint && n.path == path)
            .cloned())
    }
    async fn field_notes(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FieldNote>> {
        Ok(self
            .view
            .fields
            .iter()
            .filter(|n| n.endpoint == endpoint)
            .cloned()
            .collect())
    }
    async fn put_field_note(&mut self, _: ProjectId, note: &FieldNote) -> StoreResult<()> {
        replace(&mut self.view.fields, note.clone(), |old| {
            old.endpoint == note.endpoint && old.path == note.path
        });
        Ok(())
    }
    async fn remove_field_note(&mut self, endpoint: EndpointId, path: &str) -> StoreResult<()> {
        self.view
            .fields
            .retain(|n| !(n.endpoint == endpoint && n.path == path));
        Ok(())
    }
    async fn link(&mut self, id: LinkId) -> StoreResult<Option<Link>> {
        Ok(self.view.links.iter().find(|l| l.id == id).cloned())
    }
    async fn find_link(&mut self, identity: &LinkIdentity) -> StoreResult<Option<Link>> {
        Ok(self
            .view
            .links
            .iter()
            .find(|l| &l.identity() == identity)
            .cloned())
    }
    async fn links(&mut self, _: ProjectId) -> StoreResult<Vec<Link>> {
        Ok(self.view.links.clone())
    }
    async fn links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>> {
        Ok(self
            .view
            .links
            .iter()
            .filter(|l| l.from == endpoint || l.to == endpoint)
            .cloned()
            .collect())
    }
    async fn put_link(&mut self, _: ProjectId, link: &Link) -> StoreResult<()> {
        replace(&mut self.view.links, link.clone(), |old| {
            old.identity() == link.identity()
        });
        Ok(())
    }
    async fn remove_link(&mut self, id: LinkId) -> StoreResult<()> {
        self.view.links.retain(|l| l.id != id);
        Ok(())
    }
    async fn restore(&mut self, _: ProjectId, _: &Target, _: Option<&Value>) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }
}
