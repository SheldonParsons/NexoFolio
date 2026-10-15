//! One transaction spans every entity, revision and project lock.
use super::codec::unavailable;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, FolderId, LinkId, ProjectId, RoundId};
use nexofolio_contracts::field_values::{FieldValueArchive, FieldValueQuery};
use nexofolio_contracts::knowledge::{
    Candidate, CandidateDiagnostics, CandidateEntryKind, CandidateOperationHistory,
    CandidateRecovery, CandidateState, CandidateSummary, Command, CurationTask, DocumentLifecycle,
    EndpointNote, FieldNote, KnowledgeSearch, KnowledgeSearchPage, Link, Revision, Round,
    RoundState, Target,
};
use nexofolio_knowledge_contracts::{
    CandidateAppend, CandidateAppendRound, FolderRow, KnowledgeTx, LinkIdentity, PlacementRow,
    StoreResult,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub(super) struct PostgresTx {
    pub(super) tx: Transaction<'static, Postgres>,
    pub(super) apply_buffer: Option<super::apply_buffer::ApplyBuffer>,
}

#[async_trait]
impl KnowledgeTx for PostgresTx {
    async fn review_field_notes(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        endpoint: EndpointId,
        keys: &[String],
    ) -> StoreResult<Vec<FieldNote>> {
        self.db_review_field_notes(project, id, endpoint, keys)
            .await
    }

    async fn snapshot_fields(
        &mut self,
        project: ProjectId,
        endpoints: &[EndpointId],
    ) -> StoreResult<Vec<FieldNote>> {
        let owned: std::collections::HashSet<_> = endpoints.iter().copied().collect();
        Ok(self
            .db_project_field_notes(project)
            .await?
            .into_iter()
            .filter(|f| owned.contains(&f.endpoint))
            .collect())
    }

    async fn prepare_application(
        &mut self,
        project: ProjectId,
        view: &nexofolio_contracts::knowledge::KnowledgeSnapshot,
    ) -> StoreResult<()> {
        self.apply_buffer = Some(super::apply_buffer::ApplyBuffer::new(project, view));
        Ok(())
    }
    async fn flush_application(&mut self) -> StoreResult<()> {
        self.flush_apply().await
    }

    async fn review_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        query: &nexofolio_contracts::knowledge::CandidateReviewQuery,
    ) -> StoreResult<Option<Value>> {
        self.db_review_candidate(project, id, query).await
    }
    async fn candidate_application(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        self.db_candidate_application(project, id).await
    }
    async fn put_candidate_decision(&mut self, c: &Candidate) -> StoreResult<()> {
        self.db_put_candidate_decision(c).await
    }

    async fn search(&mut self, query: &KnowledgeSearch) -> StoreResult<KnowledgeSearchPage> {
        self.db_search(query).await
    }

    async fn candidate_input(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<Value>> {
        self.db_candidate_input(project, candidate).await
    }
    async fn frozen_chunk(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        hash: &str,
    ) -> StoreResult<Option<String>> {
        self.db_frozen_chunk(project, candidate, snapshot, hash)
            .await
    }
    async fn put_frozen_chunk(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        hash: &str,
        content: &str,
    ) -> StoreResult<()> {
        self.db_put_frozen_chunk(project, candidate, snapshot, hash, content)
            .await
    }

    async fn candidate_task(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<CurationTask>> {
        self.db_candidate_task(project, candidate).await
    }
    async fn task(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<CurationTask>> {
        self.db_task(project, id).await
    }
    async fn active_tasks(&mut self, project: ProjectId) -> StoreResult<Vec<CurationTask>> {
        self.db_active_tasks(project).await
    }
    async fn put_task(&mut self, task: &CurationTask) -> StoreResult<()> {
        self.db_put_task(task).await
    }
    async fn next_task(
        &mut self,
        now: DateTime<Utc>,
        capacity: usize,
    ) -> StoreResult<Option<CurationTask>> {
        self.db_next_task(now, capacity).await
    }
    async fn valid_worker(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.db_valid_worker(project, candidate, owner, now).await
    }
    async fn close_candidate_tasks(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.db_close_candidate_tasks(project, candidate, now).await
    }

    async fn lifecycle(&mut self, project: ProjectId) -> StoreResult<DocumentLifecycle> {
        self.db_lifecycle(project).await
    }
    async fn put_lifecycle(
        &mut self,
        project: ProjectId,
        state: &DocumentLifecycle,
    ) -> StoreResult<()> {
        self.db_put_lifecycle(project, state).await
    }

    async fn recovery_candidates(
        &mut self,
        before: DateTime<Utc>,
        after: Option<uuid::Uuid>,
        limit: usize,
    ) -> StoreResult<Vec<CandidateRecovery>> {
        self.db_recovery_candidates(before, after, limit).await
    }

    async fn value_archive_field(
        &mut self,
        q: &FieldValueQuery,
        value_id: Option<&str>,
    ) -> StoreResult<Option<FieldValueArchive>> {
        self.db_value_archive_field(q, value_id).await
    }

    async fn value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
    ) -> StoreResult<Option<FieldValueArchive>> {
        self.db_value_archive(project, candidate).await
    }
    async fn put_value_archive(
        &mut self,
        project: ProjectId,
        candidate: uuid::Uuid,
        archive: &FieldValueArchive,
    ) -> StoreResult<()> {
        self.db_put_value_archive(project, candidate, archive).await
    }

    async fn active_candidate(
        &mut self,
        project: ProjectId,
    ) -> StoreResult<Option<CandidateSummary>> {
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
    ) -> StoreResult<Option<CandidateState>> {
        self.db_candidate_state(project, id).await
    }
    async fn checkpoint_candidate(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        state: CandidateState,
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
    ) -> StoreResult<Option<Candidate>> {
        self.db_candidate(project, id).await
    }
    async fn candidate_diagnostics(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<CandidateDiagnostics>> {
        self.db_candidate_diagnostics(project, id).await
    }
    async fn candidate_projection(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<Candidate>> {
        self.db_candidate_projection(project, id).await
    }
    async fn candidate_append(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
        round: RoundId,
    ) -> StoreResult<Option<CandidateAppend>> {
        self.db_candidate_append(project, id, round).await
    }
    async fn append_candidate_commands(
        &mut self,
        context: &CandidateAppend,
        round: &CandidateAppendRound,
        proposed: &[Command],
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.db_append_candidate_commands(context, round, proposed, at)
            .await
    }
    async fn put_candidate_projection(&mut self, candidate: &Candidate) -> StoreResult<()> {
        self.db_put_candidate_projection(candidate).await
    }
    async fn candidates(
        &mut self,
        project: ProjectId,
        limit: usize,
    ) -> StoreResult<Vec<CandidateSummary>> {
        self.db_candidates(project, limit).await
    }
    async fn candidate_operations(
        &mut self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> StoreResult<Option<CandidateOperationHistory>> {
        self.db_candidate_operations(project, id).await
    }
    async fn entry_candidates(
        &mut self,
        project: ProjectId,
        kind: CandidateEntryKind,
        selected: Option<uuid::Uuid>,
    ) -> StoreResult<Vec<CandidateSummary>> {
        self.db_entry_candidates(project, kind, selected).await
    }
    async fn put_candidate(&mut self, candidate: &Candidate) -> StoreResult<()> {
        self.db_put_candidate(candidate).await
    }
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        self.tx.commit().await.map_err(unavailable)
    }

    async fn lock_project(&mut self, project: ProjectId) -> StoreResult<()> {
        self.db_lock_project(project).await
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

    async fn project_field_notes(&mut self, project: ProjectId) -> StoreResult<Vec<FieldNote>> {
        self.db_project_field_notes(project).await
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
