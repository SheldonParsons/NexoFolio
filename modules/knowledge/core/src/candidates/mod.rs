//! Durable proposals, reuse and atomic review. Never interpreted by official reads.
mod batch_writes;
mod create;
mod decisions;
mod inheritance;
mod recovery;
mod session;
mod snapshot;
mod snapshot_tx;
mod validation;
mod value_archive;
pub use session::CandidateSession;
use validation::{commands, load, running, valid_names};

use crate::{Clock, Knowledge, store_error};
use async_trait::async_trait;
use nexofolio_common::{EndpointId, ProjectId, RoundId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;
use snapshot_tx::SnapshotTx;

#[async_trait]
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> KnowledgeCandidates for Knowledge<S, R, C> {
    async fn freeze_candidate_values(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        archive: &nexofolio_contracts::field_values::FieldValueArchive,
    ) -> KnowledgeResult<()> {
        self.freeze_values(project, id, owner, archive).await
    }

    async fn create_candidate_as(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate> {
        self.create_proposal_as(project, target, parent, actor)
            .await
    }
    async fn resume_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: serde_json::Value,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate> {
        self.resume_proposal(project, id, owner, input, actor).await
    }
    async fn freeze_candidate_input(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: serde_json::Value,
    ) -> KnowledgeResult<()> {
        self.freeze_input(project, id, owner, input).await
    }
    async fn cancel_candidate(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<()> {
        self.cancel_proposal(project, id).await
    }
    async fn heartbeat_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
    ) -> KnowledgeResult<()> {
        self.heartbeat(project, id, owner).await
    }
    async fn candidate(&self, project: ProjectId, id: uuid::Uuid) -> KnowledgeResult<Candidate> {
        load(
            self.store.begin().await.map_err(store_error)?.as_mut(),
            project,
            id,
        )
        .await
    }
    async fn active_candidate(
        &self,
        project: ProjectId,
    ) -> KnowledgeResult<Option<CandidateSummary>> {
        self.store
            .begin()
            .await
            .map_err(store_error)?
            .active_candidate(project)
            .await
            .map_err(store_error)
    }
    async fn candidate_operations(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> KnowledgeResult<CandidateOperationHistory> {
        self.store
            .begin()
            .await
            .map_err(store_error)?
            .candidate_operations(project, id)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)
    }
    async fn candidates(
        &self,
        project: ProjectId,
        limit: usize,
    ) -> KnowledgeResult<Vec<CandidateSummary>> {
        self.store
            .begin()
            .await
            .map_err(store_error)?
            .candidates(project, limit.clamp(1, 50))
            .await
            .map_err(store_error)
    }
    async fn entry_candidates(
        &self,
        project: ProjectId,
        kind: CandidateEntryKind,
        selected: Option<uuid::Uuid>,
    ) -> KnowledgeResult<Vec<CandidateSummary>> {
        self.store
            .begin()
            .await
            .map_err(store_error)?
            .entry_candidates(project, kind, selected)
            .await
            .map_err(store_error)
    }
    async fn open_candidate_batch(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        author: Author,
    ) -> KnowledgeResult<RoundId> {
        if author.is_person() {
            return Err(KnowledgeError::ProtectedKnowledge);
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = validation::load_projection(tx.as_mut(), project, id).await?;
        running(&candidate)?;
        recovery::owned(&candidate, owner)?;
        let batch = RoundId::new();
        candidate.batches.push(CandidateBatch {
            id: batch,
            author,
            state: RoundState::Running,
            commands: vec![],
        });
        candidate.updated_at = self.clock.now();
        tx.put_candidate_projection(&candidate)
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(batch)
    }
    async fn append_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        proposed: &[Command],
    ) -> KnowledgeResult<Candidate> {
        self.append_proposal(project, id, owner, batch, proposed, false)
            .await
    }
    async fn append_candidate_view(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        proposed: &[Command],
    ) -> KnowledgeResult<KnowledgeSnapshot> {
        Ok(self
            .append_proposal(project, id, owner, batch, proposed, true)
            .await?
            .view)
    }
    async fn close_candidate_batch(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        state: RoundState,
    ) -> KnowledgeResult<()> {
        if state == RoundState::Running {
            return Err(KnowledgeError::RoundClosed);
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = validation::load_projection(tx.as_mut(), project, id).await?;
        running(&candidate)?;
        recovery::owned(&candidate, owner)?;
        let group = candidate
            .batches
            .iter_mut()
            .find(|b| b.id == batch && b.state == RoundState::Running)
            .ok_or(KnowledgeError::RoundClosed)?;
        group.state = state;
        candidate.updated_at = self.clock.now();
        tx.put_candidate_projection(&candidate)
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    async fn checkpoint_candidate(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        state: CandidateState,
        progress: serde_json::Value,
    ) -> KnowledgeResult<()> {
        if !matches!(
            state,
            CandidateState::Running
                | CandidateState::Ready
                | CandidateState::Partial
                | CandidateState::Interrupted
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        match tx.candidate_state(project, id).await.map_err(store_error)? {
            None => return Err(KnowledgeError::NoSuchCandidate),
            Some(CandidateState::Running) => {}
            _ => return Err(KnowledgeError::CandidateClosed),
        }
        let owner = progress
            .get("run_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());
        if tx
            .candidate_owner(project, id)
            .await
            .map_err(store_error)?
            .is_some_and(|expected| Some(expected) != owner)
        {
            return Err(KnowledgeError::CandidateClosed);
        }
        tx.checkpoint_candidate(project, id, state, &progress, self.clock.now())
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    async fn apply_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<Vec<RoundId>> {
        self.apply_proposal_as(project, id, actor).await
    }
    async fn discard_candidate_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<()> {
        self.discard_proposal_as(project, id, actor).await
    }
}
