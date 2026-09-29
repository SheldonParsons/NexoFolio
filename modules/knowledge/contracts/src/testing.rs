//! In-memory knowledge store, so core can be tested without a database.
//!
//! It keeps rows in maps and reproduces only what core relies on: one placement
//! per endpoint, link identity being the two ends plus the relation, revisions
//! newest first, and an uncommitted transaction leaving nothing behind. The
//! Postgres store is checked separately against a real database.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::{
    Author, EndpointNote, FieldNote, Link, Relation, Revision, Round, RoundState, Target,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    FolderRow, KnowledgeStore, KnowledgeTx, LinkIdentity, PlacementRow, StoreError, StoreResult,
};

#[derive(Clone, Default)]
struct State {
    rounds: HashMap<RoundId, Round>,
    revisions: Vec<Revision>,
    folders: HashMap<FolderId, FolderRow>,
    placements: HashMap<EndpointId, PlacementRow>,
    endpoint_notes: HashMap<EndpointId, (ProjectId, EndpointNote)>,
    field_notes: HashMap<(EndpointId, String), (ProjectId, FieldNote)>,
    links: HashMap<LinkId, (ProjectId, Link)>,
}

impl State {
    fn identity(link: &Link) -> LinkIdentity {
        LinkIdentity {
            from: link.from,
            from_field: link.from_field.clone(),
            to: link.to,
            to_field: link.to_field.clone(),
            relation: link.relation,
        }
    }

    /// Folders of a project reachable from `id` downwards, for cascading removal.
    fn subtree(&self, id: FolderId) -> Vec<FolderId> {
        let mut found = vec![id];
        let mut at = 0;
        while at < found.len() {
            let parent = found[at];
            for row in self.folders.values() {
                if row.parent == Some(parent) && !found.contains(&row.id) {
                    found.push(row.id);
                }
            }
            at += 1;
        }
        found
    }
}

/// A fake store. Clones share the same rows, so a test can hold one and hand
/// another to the code under test.
#[derive(Clone, Default)]
pub struct InMemoryKnowledge {
    state: Arc<Mutex<State>>,
}

impl InMemoryKnowledge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Committed rounds only, newest first.
    pub fn rounds(&self) -> Vec<Round> {
        let state = self.state.lock().expect("fake lock");
        let mut rounds: Vec<Round> = state.rounds.values().cloned().collect();
        rounds.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        rounds
    }

    pub fn folders(&self) -> Vec<FolderRow> {
        self.state
            .lock()
            .expect("fake lock")
            .folders
            .values()
            .cloned()
            .collect()
    }

    pub fn links(&self) -> Vec<Link> {
        self.state
            .lock()
            .expect("fake lock")
            .links
            .values()
            .map(|(_, link)| link.clone())
            .collect()
    }

    pub fn revisions(&self) -> Vec<Revision> {
        self.state.lock().expect("fake lock").revisions.clone()
    }
}

#[async_trait]
impl KnowledgeStore for InMemoryKnowledge {
    async fn begin(&self) -> StoreResult<Box<dyn KnowledgeTx>> {
        // A copy of the rows: dropping it without committing discards the copy.
        let working = self.state.lock().expect("fake lock").clone();
        Ok(Box::new(InMemoryTx {
            shared: Arc::clone(&self.state),
            working,
        }))
    }
}

struct InMemoryTx {
    shared: Arc<Mutex<State>>,
    working: State,
}

#[async_trait]
impl KnowledgeTx for InMemoryTx {
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        *self.shared.lock().expect("fake lock") = self.working;
        Ok(())
    }

    /// Nothing to serialise: the fake is not concurrent.
    async fn lock_project(&mut self, _project: ProjectId) -> StoreResult<()> {
        Ok(())
    }

    async fn create_round(&mut self, round: &Round) -> StoreResult<()> {
        self.working.rounds.insert(round.id, round.clone());
        Ok(())
    }

    async fn round(&mut self, round: RoundId) -> StoreResult<Option<Round>> {
        Ok(self.working.rounds.get(&round).cloned())
    }

    async fn finish_round(
        &mut self,
        round: RoundId,
        state: RoundState,
        summary: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        if let Some(open) = self.working.rounds.get_mut(&round) {
            open.state = state;
            open.summary = summary.map(str::to_owned);
            open.finished_at = Some(at);
        }
        Ok(())
    }

    async fn rounds(&mut self, project: ProjectId, limit: usize) -> StoreResult<Vec<Round>> {
        let mut rounds: Vec<Round> = self
            .working
            .rounds
            .values()
            .filter(|r| r.project_id == project)
            .cloned()
            .collect();
        rounds.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        rounds.truncate(limit);
        Ok(rounds)
    }

    async fn record_revision(&mut self, revision: &Revision) -> StoreResult<()> {
        self.working.revisions.push(revision.clone());
        Ok(())
    }

    async fn revisions(&mut self, round: RoundId) -> StoreResult<Vec<Revision>> {
        let mut found: Vec<Revision> = self
            .working
            .revisions
            .iter()
            .filter(|r| r.round == round)
            .cloned()
            .collect();
        // Newest first: the order a rollback must replay them in.
        found.sort_by(|a, b| b.seq.cmp(&a.seq));
        Ok(found)
    }

    async fn folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>> {
        Ok(self.working.folders.get(&id).cloned())
    }

    async fn folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>> {
        Ok(self
            .working
            .folders
            .values()
            .filter(|f| f.project_id == project)
            .cloned()
            .collect())
    }

    async fn put_folder(&mut self, folder: &FolderRow) -> StoreResult<()> {
        self.working.folders.insert(folder.id, folder.clone());
        Ok(())
    }

    async fn remove_folder(&mut self, id: FolderId) -> StoreResult<()> {
        // Sub-folders go with it, and so do the placements inside them.
        for gone in self.working.subtree(id) {
            self.working.folders.remove(&gone);
            self.working.placements.retain(|_, p| p.folder != gone);
        }
        Ok(())
    }

    async fn placement(&mut self, endpoint: EndpointId) -> StoreResult<Option<PlacementRow>> {
        Ok(self.working.placements.get(&endpoint).cloned())
    }

    async fn placements(&mut self, project: ProjectId) -> StoreResult<Vec<PlacementRow>> {
        Ok(self
            .working
            .placements
            .values()
            .filter(|p| p.project_id == project)
            .cloned()
            .collect())
    }

    async fn put_placement(&mut self, placement: &PlacementRow) -> StoreResult<()> {
        // One row per endpoint: placing it again moves it.
        self.working
            .placements
            .insert(placement.endpoint, placement.clone());
        Ok(())
    }

    async fn remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()> {
        self.working.placements.remove(&endpoint);
        Ok(())
    }

    async fn endpoint_note(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointNote>> {
        Ok(self
            .working
            .endpoint_notes
            .get(&endpoint)
            .map(|(_, note)| note.clone()))
    }

    async fn endpoint_notes(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointNote>> {
        Ok(self
            .working
            .endpoint_notes
            .values()
            .filter(|(owner, _)| *owner == project)
            .map(|(_, note)| note.clone())
            .collect())
    }

    async fn put_endpoint_note(
        &mut self,
        project: ProjectId,
        note: &EndpointNote,
    ) -> StoreResult<()> {
        self.working
            .endpoint_notes
            .insert(note.endpoint, (project, note.clone()));
        Ok(())
    }

    async fn field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>> {
        Ok(self
            .working
            .field_notes
            .get(&(endpoint, path.to_owned()))
            .map(|(_, note)| note.clone()))
    }

    async fn field_notes(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FieldNote>> {
        let mut found: Vec<FieldNote> = self
            .working
            .field_notes
            .iter()
            .filter(|((owner, _), _)| *owner == endpoint)
            .map(|(_, (_, note))| note.clone())
            .collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }

    async fn put_field_note(&mut self, project: ProjectId, note: &FieldNote) -> StoreResult<()> {
        self.working
            .field_notes
            .insert((note.endpoint, note.path.clone()), (project, note.clone()));
        Ok(())
    }

    async fn remove_field_note(&mut self, endpoint: EndpointId, path: &str) -> StoreResult<()> {
        self.working
            .field_notes
            .remove(&(endpoint, path.to_owned()));
        Ok(())
    }

    async fn link(&mut self, id: LinkId) -> StoreResult<Option<Link>> {
        Ok(self.working.links.get(&id).map(|(_, link)| link.clone()))
    }

    async fn find_link(&mut self, identity: &LinkIdentity) -> StoreResult<Option<Link>> {
        Ok(self
            .working
            .links
            .values()
            .find(|(_, link)| State::identity(link) == *identity)
            .map(|(_, link)| link.clone()))
    }

    async fn links(&mut self, project: ProjectId) -> StoreResult<Vec<Link>> {
        Ok(self
            .working
            .links
            .values()
            .filter(|(owner, _)| *owner == project)
            .map(|(_, link)| link.clone())
            .collect())
    }

    async fn links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>> {
        Ok(self
            .working
            .links
            .values()
            .filter(|(_, link)| link.from == endpoint || link.to == endpoint)
            .map(|(_, link)| link.clone())
            .collect())
    }

    async fn put_link(&mut self, project: ProjectId, link: &Link) -> StoreResult<()> {
        // Identity is the ends plus the relation: rediscovery refreshes the
        // existing row rather than adding a duplicate, keeping its id.
        let identity = State::identity(link);
        let existing = self
            .working
            .links
            .iter()
            .find(|(_, (_, other))| State::identity(other) == identity)
            .map(|(id, _)| *id);
        let id = existing.unwrap_or(link.id);
        self.working
            .links
            .insert(id, (project, Link { id, ..link.clone() }));
        Ok(())
    }

    async fn remove_link(&mut self, id: LinkId) -> StoreResult<()> {
        self.working.links.remove(&id);
        Ok(())
    }

    async fn restore(
        &mut self,
        project: ProjectId,
        target: &Target,
        before: Option<&Value>,
    ) -> StoreResult<()> {
        fn kept<T: for<'a> Deserialize<'a>>(value: &Value) -> StoreResult<T> {
            serde_json::from_value(value.clone()).map_err(|_| StoreError::Unavailable)
        }

        match (target, before) {
            // Nothing was there before the write, so undoing it removes the row.
            (Target::Folder { id }, None) => self.remove_folder(*id).await,
            (Target::Placement { endpoint }, None) => self.remove_placement(*endpoint).await,
            (Target::EndpointNote { endpoint }, None) => {
                self.working.endpoint_notes.remove(endpoint);
                Ok(())
            }
            (Target::FieldNote { endpoint, path }, None) => {
                self.remove_field_note(*endpoint, path).await
            }
            (Target::Link { id }, None) => self.remove_link(*id).await,

            (Target::Folder { id }, Some(before)) => {
                let row: KeptFolder = kept(before)?;
                self.put_folder(&FolderRow {
                    id: *id,
                    project_id: project,
                    parent: row.parent,
                    name: row.name,
                    summary: row.summary,
                    position: row.position,
                })
                .await
            }
            (Target::Placement { endpoint }, Some(before)) => {
                let row: KeptPlacement = kept(before)?;
                self.put_placement(&PlacementRow {
                    endpoint: *endpoint,
                    project_id: project,
                    folder: row.folder,
                    author: row.author,
                    at: row.at,
                })
                .await
            }
            (Target::EndpointNote { endpoint }, Some(before)) => {
                let row: KeptEndpointNote = kept(before)?;
                self.put_endpoint_note(
                    project,
                    &EndpointNote {
                        endpoint: *endpoint,
                        name: row.name,
                        purpose: row.purpose,
                        author: row.author,
                        at: row.at,
                    },
                )
                .await
            }
            (Target::FieldNote { endpoint, path }, Some(before)) => {
                let row: KeptFieldNote = kept(before)?;
                self.put_field_note(
                    project,
                    &FieldNote {
                        endpoint: *endpoint,
                        path: path.clone(),
                        text: row.text,
                        values: row.values,
                        author: row.author,
                        at: row.at,
                    },
                )
                .await
            }
            (Target::Link { id }, Some(before)) => {
                let row: KeptLink = kept(before)?;
                self.put_link(
                    project,
                    &Link {
                        id: *id,
                        from: row.from,
                        from_field: row.from_field,
                        to: row.to,
                        to_field: row.to_field,
                        relation: row.relation,
                        evidence: row.evidence,
                        author: row.author,
                        at: row.at,
                    },
                )
                .await
            }
        }
    }
}

// What a revision keeps per target, as core writes it.

#[derive(Deserialize)]
struct KeptFolder {
    parent: Option<FolderId>,
    name: String,
    summary: Option<String>,
    position: i32,
}

#[derive(Deserialize)]
struct KeptPlacement {
    folder: FolderId,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct KeptEndpointNote {
    name: Option<String>,
    purpose: Option<String>,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct KeptFieldNote {
    text: String,
    values: Vec<String>,
    author: Author,
    at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct KeptLink {
    from: EndpointId,
    from_field: Option<String>,
    to: EndpointId,
    to_field: Option<String>,
    relation: Relation,
    evidence: String,
    author: Author,
    at: DateTime<Utc>,
}
