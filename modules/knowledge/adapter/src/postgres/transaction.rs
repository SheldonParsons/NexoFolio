//! One transaction spans every entity, revision and project lock.
use super::codec::unavailable;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId};
use nexofolio_contracts::knowledge::{
    EndpointNote, FieldNote, Link, Revision, Round, RoundState, Target,
};
use nexofolio_knowledge_contracts::{
    FolderRow, KnowledgeTx, LinkIdentity, PlacementRow, StoreResult,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

const PROJECT_LOCK: i32 = 0x6b6e_0001;

pub(super) struct PostgresTx {
    pub(super) tx: Transaction<'static, Postgres>,
}

#[async_trait]
impl KnowledgeTx for PostgresTx {
    async fn value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::field_values::FieldValueArchive>> {
        self.db_value_archive(project, candidate).await
    }
    async fn put_value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        archive: &nexofolio_contracts::field_values::FieldValueArchive,
    ) -> StoreResult<()> {
        self.db_put_value_archive(project, candidate, archive).await
    }

    async fn active_candidate(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateSummary>> {
        self.db_active_candidate(project).await
    }
    async fn candidate_owner(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<uuid::Uuid>> {
        self.db_candidate_owner(project, id).await
    }
    async fn heartbeat_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<()> {
        self.db_heartbeat(project, id, at).await
    }
    async fn candidate_state(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateState>> {
        self.db_candidate_state(project, id).await
    }
    async fn checkpoint_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        state: nexofolio_contracts::knowledge::CandidateState,
        progress: &Value,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.db_checkpoint_candidate(project, id, state, progress, at)
            .await
    }
    async fn candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::Candidate>> {
        self.db_candidate(project, id).await
    }
    async fn candidate_projection(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::Candidate>> {
        self.db_candidate_projection(project, id).await
    }
    async fn put_candidate_projection(
        &mut self,
        candidate: &nexofolio_contracts::knowledge::Candidate,
    ) -> StoreResult<()> {
        self.db_put_candidate_projection(candidate).await
    }
    async fn candidates(
        &mut self,
        project: ProjectId,
        limit: usize,
    ) -> StoreResult<Vec<nexofolio_contracts::knowledge::CandidateSummary>> {
        self.db_candidates(project, limit).await
    }
    async fn candidate_operations(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<nexofolio_contracts::knowledge::CandidateOperationHistory>> {
        self.db_candidate_operations(project, id).await
    }
    async fn entry_candidates(
        &mut self,
        project: ProjectId,
        kind: nexofolio_contracts::knowledge::CandidateEntryKind,
        selected: Option<uuid::Uuid>,
    ) -> StoreResult<Vec<nexofolio_contracts::knowledge::CandidateSummary>> {
        self.db_entry_candidates(project, kind, selected).await
    }
    async fn put_candidate(
        &mut self,
        candidate: &nexofolio_contracts::knowledge::Candidate,
    ) -> StoreResult<()> {
        self.db_put_candidate(candidate).await
    }
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        self.tx.commit().await.map_err(unavailable)
    }

    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()> {
        sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
            .bind(PROJECT_LOCK)
            .bind(project.to_string())
            .execute(&mut *self.tx)
            .await
            .map(|_| ())
            .map_err(unavailable)
    }

    async fn create_round(&mut self, round: &Round) -> StoreResult<()> {
        self.db_create_round(round).await
    }

    async fn round(&mut self, round: RoundId) -> StoreResult<Option<Round>> {
        self.db_round(round).await
    }

    async fn finish_round(
        &mut self,
        round: RoundId,
        state: RoundState,
        summary: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.db_finish_round(round, state, summary, at).await
    }

    async fn rounds(&mut self, project: ProjectId, limit: usize) -> StoreResult<Vec<Round>> {
        self.db_rounds(project, limit).await
    }

    async fn record_revision(&mut self, revision: &Revision) -> StoreResult<()> {
        self.db_record_revision(revision).await
    }

    async fn revisions(&mut self, round: RoundId) -> StoreResult<Vec<Revision>> {
        self.db_revisions(round).await
    }

    async fn folder(&mut self, id: FolderId) -> StoreResult<Option<FolderRow>> {
        self.db_folder(id).await
    }

    async fn folders(&mut self, project: ProjectId) -> StoreResult<Vec<FolderRow>> {
        self.db_folders(project).await
    }

    async fn put_folder(&mut self, folder: &FolderRow) -> StoreResult<()> {
        self.db_put_folder(folder).await
    }

    async fn remove_folder(&mut self, id: FolderId) -> StoreResult<()> {
        self.db_remove_folder(id).await
    }

    async fn placement(&mut self, endpoint: EndpointId) -> StoreResult<Option<PlacementRow>> {
        self.db_placement(endpoint).await
    }

    async fn placements(&mut self, project: ProjectId) -> StoreResult<Vec<PlacementRow>> {
        self.db_placements(project).await
    }

    async fn put_placement(&mut self, placement: &PlacementRow) -> StoreResult<()> {
        self.db_put_placement(placement).await
    }

    async fn remove_placement(&mut self, endpoint: EndpointId) -> StoreResult<()> {
        self.db_remove_placement(endpoint).await
    }

    async fn endpoint_note(&mut self, endpoint: EndpointId) -> StoreResult<Option<EndpointNote>> {
        self.db_endpoint_note(endpoint).await
    }

    async fn endpoint_notes(&mut self, project: ProjectId) -> StoreResult<Vec<EndpointNote>> {
        self.db_endpoint_notes(project).await
    }

    async fn put_endpoint_note(
        &mut self,
        project: ProjectId,
        note: &EndpointNote,
    ) -> StoreResult<()> {
        self.db_put_endpoint_note(project, note).await
    }

    async fn field_note(
        &mut self,
        endpoint: EndpointId,
        path: &str,
    ) -> StoreResult<Option<FieldNote>> {
        self.db_field_note(endpoint, path).await
    }

    async fn field_notes(&mut self, endpoint: EndpointId) -> StoreResult<Vec<FieldNote>> {
        self.db_field_notes(endpoint).await
    }

    async fn put_field_note(&mut self, project: ProjectId, note: &FieldNote) -> StoreResult<()> {
        self.db_put_field_note(project, note).await
    }

    async fn remove_field_note(&mut self, endpoint: EndpointId, path: &str) -> StoreResult<()> {
        self.db_remove_field_note(endpoint, path).await
    }

    async fn link(&mut self, id: LinkId) -> StoreResult<Option<Link>> {
        self.db_link(id).await
    }

    async fn find_link(&mut self, identity: &LinkIdentity) -> StoreResult<Option<Link>> {
        self.db_find_link(identity).await
    }

    async fn links(&mut self, project: ProjectId) -> StoreResult<Vec<Link>> {
        self.db_links(project).await
    }

    async fn links_of(&mut self, endpoint: EndpointId) -> StoreResult<Vec<Link>> {
        self.db_links_of(endpoint).await
    }

    async fn put_link(&mut self, project: ProjectId, link: &Link) -> StoreResult<()> {
        self.db_put_link(project, link).await
    }

    async fn remove_link(&mut self, id: LinkId) -> StoreResult<()> {
        self.db_remove_link(id).await
    }

    async fn restore(
        &mut self,
        project: ProjectId,
        target: &Target,
        before: Option<&Value>,
    ) -> StoreResult<()> {
        self.db_restore(project, target, before).await
    }
}
