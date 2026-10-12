//! What knowledge core and its storage adapter share: the rows knowledge keeps
//! and the transactional store that keeps them.
//!
//! Core decides everything (tree rules, depth, what a command means, what the
//! previous value was); the store only remembers. One [`KnowledgeTx`] covers one
//! `apply` call, so a batch of commands lands whole or not at all.
//!
//! The public contract is [`nexofolio_contracts::knowledge`]; these types exist
//! so the store can persist it without core reaching into SQL.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId};
pub use nexofolio_contracts::knowledge::LinkIdentity;
use nexofolio_contracts::knowledge::{
    Author, Candidate, CandidateSummary, EndpointNote, FieldNote, Link, Revision, Round, RoundState,
};
use serde_json::Value;

#[cfg(feature = "testing")]
pub mod testing;

/// A folder as stored: no counts, those are computed when the tree is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    pub id: FolderId,
    pub author: Option<Author>,
    pub project_id: ProjectId,
    pub parent: Option<FolderId>,
    pub name: String,
    pub summary: Option<String>,
    pub position: i32,
}

/// Where one endpoint sits, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementRow {
    pub endpoint: EndpointId,
    pub project_id: ProjectId,
    pub folder: FolderId,
    pub author: Author,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Retryable: storage is down.
    #[error("knowledge storage unavailable")]
    Unavailable,
}

pub type StoreResult<T> = Result<T, StoreError>;

#[async_trait]
pub trait KnowledgeStore: Send + Sync {
    /// Dropping the transaction without [`KnowledgeTx::commit`] undoes it.
    async fn begin(&self) -> StoreResult<Box<dyn KnowledgeTx>>;

    /// Consistent read snapshot for compound projections. Adapters must not
    /// acquire writer locks or advance domain state.
    async fn begin_read(&self) -> StoreResult<Box<dyn KnowledgeTx>> {
        self.begin().await
    }
}

#[async_trait]
pub trait KnowledgeTx: Send {
    async fn lifecycle(
        &mut self,
        _project: ProjectId,
    ) -> StoreResult<nexofolio_contracts::knowledge::DocumentLifecycle> {
        Ok(Default::default())
    }
    async fn put_lifecycle(
        &mut self,
        _project: ProjectId,
        _state: &nexofolio_contracts::knowledge::DocumentLifecycle,
    ) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }

    async fn recovery_candidates(
        &mut self,
        _before: DateTime<Utc>,
        _after: Option<uuid::Uuid>,
        _limit: usize,
    ) -> StoreResult<Vec<nexofolio_contracts::knowledge::CandidateRecovery>> {
        Err(StoreError::Unavailable)
    }

    async fn value_archive(
        &mut self,
        _project: ProjectId,
        _candidate: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::field_values::FieldValueArchive>> {
        Ok(None)
    }
    async fn put_value_archive(
        &mut self,
        _project: ProjectId,
        _candidate: uuid::Uuid,
        _archive: &nexofolio_contracts::field_values::FieldValueArchive,
    ) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }

    async fn active_candidate(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Option<CandidateSummary>>;
    async fn candidate_owner(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<uuid::Uuid>> {
        Ok(self.candidate(project, id).await?.and_then(|c| {
            c.progress
                .get("run_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        }))
    }
    async fn heartbeat_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        let mut c = self
            .candidate(project, id)
            .await?
            .ok_or(StoreError::Unavailable)?;
        c.updated_at = at;
        self.put_candidate(&c).await
    }

    async fn candidate_state(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateState>> {
        Ok(self.candidate(project, id).await?.map(|c| c.state))
    }
    async fn checkpoint_candidate(
        &mut self,
        _project: ProjectId,
        _id: uuid::Uuid,
        _state: nexofolio_contracts::knowledge::CandidateState,
        _progress: &Value,
        _at: DateTime<Utc>,
    ) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }
    async fn candidate(
        &mut self,
        _project: ProjectId,
        _id: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        Err(StoreError::Unavailable)
    }
    async fn candidate_diagnostics(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateDiagnostics>> {
        Ok(self.candidate(project, id).await?.map(Into::into))
    }
    /// Mutable working projection; deliberately omits the immutable execution input.
    async fn candidate_projection(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        let mut candidate = self.candidate(project, id).await?;
        if let Some(c) = &mut candidate {
            c.execution_input = None;
        }
        Ok(candidate)
    }
    /// Update a working projection without replacing its immutable execution input.
    async fn put_candidate_projection(&mut self, candidate: &Candidate) -> StoreResult<()> {
        let mut updated = candidate.clone();
        updated.execution_input = self
            .candidate(candidate.project_id, candidate.id)
            .await?
            .ok_or(StoreError::Unavailable)?
            .execution_input;
        self.put_candidate(&updated).await
    }
    async fn candidates(
        &mut self,
        _project: ProjectId,
        _limit: usize,
    ) -> StoreResult<Vec<CandidateSummary>> {
        Err(StoreError::Unavailable)
    }
    async fn candidate_operations(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateOperationHistory>> {
        Ok(self
            .candidate(project, id)
            .await?
            .map(|c| c.operation_history()))
    }
    async fn put_candidate(&mut self, _candidate: &Candidate) -> StoreResult<()> {
        Err(StoreError::Unavailable)
    }
    async fn entry_candidates(
        &mut self,
        _project: ProjectId,
        _kind: nexofolio_contracts::knowledge::CandidateEntryKind,
        _selected: Option<uuid::Uuid>,
    ) -> StoreResult<Vec<CandidateSummary>> {
        Err(StoreError::Unavailable)
    }
    async fn commit(self: Box<Self>) -> StoreResult<()>;

    /// Serialises writers of one project until the transaction ends, so two
    /// rounds cannot interleave their tree edits.
    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()>;

    // -- rounds ---------------------------------------------------------------

    async fn create_round(&mut self, round: &Round) -> StoreResult<()>;

    async fn round(&mut self, round: RoundId) -> StoreResult<Option<Round>>;

    async fn finish_round(
        &mut self,
        round: RoundId,
        state: RoundState,
        summary: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()>;

    /// Most recent first.
    async fn rounds(&mut self, project: ProjectId, limit: usize) -> StoreResult<Vec<Round>>;

    /// Appends one revision; `seq` orders writes inside a round so a rollback
    /// can undo them newest first.
    async fn record_revision(&mut self, revision: &Revision) -> StoreResult<()>;

    /// Newest write first, which is the order a rollback must replay them in.
    async fn revisions(&mut self, round: RoundId) -> StoreResult<Vec<Revision>>;

    // -- folders --------------------------------------------------------------

    async fn folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>>;

    /// Every folder of a project, in no particular order; core sorts them.
    async fn folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>>;

    /// Creates it when `id` is unknown, otherwise replaces name, summary,
    /// parent and position.
    async fn put_folder(&mut self, folder: &FolderRow) -> StoreResult<()>;

    /// Also drops its sub-folders and frees the endpoints placed in any of them.
    async fn remove_folder(&mut self, id: FolderId) -> StoreResult<()>;

    // -- placements -----------------------------------------------------------

    async fn placement(&mut self, endpoint: EndpointId) -> StoreResult<Option<PlacementRow>>;

    async fn placements(&mut self, project: ProjectId) -> StoreResult<Vec<PlacementRow>>;

    /// Replaces any previous placement of the endpoint.
    async fn put_placement(&mut self, placement: &PlacementRow) -> StoreResult<()>;

    async fn remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()>;

    // -- notes ----------------------------------------------------------------

    async fn endpoint_note(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointNote>>;

    /// Endpoints with nothing written about them are left out.
    async fn endpoint_notes(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointNote>>;

    async fn put_endpoint_note(
        &mut self,
        project: ProjectId,
        note: &EndpointNote,
    ) -> StoreResult<()>;

    async fn field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>>;

    async fn field_notes(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FieldNote>>;

    async fn project_field_notes(&mut self, _project: ProjectId) -> StoreResult<Vec<FieldNote>> {
        Err(StoreError::Unavailable)
    }

    async fn put_field_note(&mut self, project: ProjectId, note: &FieldNote) -> StoreResult<()>;

    async fn remove_field_note(&mut self, endpoint: EndpointId, path: &str) -> StoreResult<()>;

    // -- links ----------------------------------------------------------------

    async fn link(&mut self, id: LinkId) -> StoreResult<Option<Link>>;

    async fn find_link(&mut self, identity: &LinkIdentity) -> StoreResult<Option<Link>>;

    async fn links(&mut self, project: ProjectId) -> StoreResult<Vec<Link>>;

    /// Links of one endpoint, in either direction.
    async fn links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>>;

    /// Creates it, or refreshes the evidence and author of the matching one.
    async fn put_link(&mut self, project: ProjectId, link: &Link) -> StoreResult<()>;

    async fn remove_link(&mut self, id: LinkId) -> StoreResult<()>;

    /// Restores a row from the JSON a revision kept, or deletes it when
    /// `before` is `None`. Core does not know the row layouts, so the store
    /// does the reverse of its own writes.
    ///
    /// `project` is needed because a revision keeps the values a write replaced,
    /// not the whole row: re-creating a deleted row needs the project back.
    async fn restore(
        &mut self,
        project: ProjectId,
        target: &nexofolio_contracts::knowledge::Target,
        before: Option<&Value>,
    ) -> StoreResult<()>;
}
