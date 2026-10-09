use super::{
    snapshot,
    validation::{commands, load},
};
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::{ProjectId, RoundId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;

impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn apply_proposal_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<Vec<RoundId>> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        if candidate.state == CandidateState::Applied {
            return Ok(candidate.applied_rounds);
        }
        if !matches!(
            candidate.state,
            CandidateState::Ready | CandidateState::Partial
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        let owned: Vec<_> = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let current = snapshot::capture(tx.as_mut(), project, owned.clone()).await?;
        if snapshot::hash(&current) != candidate.base_hash {
            return Err(KnowledgeError::CandidateStale);
        }
        let mut applied = Vec::new();
        for group in candidate.batches.iter().filter(|b| !b.commands.is_empty()) {
            let round = Round {
                id: RoundId::new(),
                project_id: project,
                author: group.author.clone(),
                started_at: self.clock.now(),
                finished_at: None,
                state: RoundState::Running,
                summary: None,
            };
            tx.create_round(&round).await.map_err(store_error)?;
            let mut writes = group.clone();
            writes.id = round.id;
            commands(tx.as_mut(), project, &owned, &writes, self.clock.now(), 0).await?;
            tx.finish_round(
                round.id,
                RoundState::Done,
                Some(&format!("应用候选整理 {}", id)),
                self.clock.now(),
            )
            .await
            .map_err(store_error)?;
            applied.push(round.id);
        }
        candidate.state = CandidateState::Applied;
        candidate.reviewed_by = actor.user_id();
        candidate.applied_rounds = applied.clone();
        candidate.updated_at = self.clock.now();
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Apply,
            actor,
            candidate.updated_at,
        ));
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(applied)
    }
    pub(super) async fn discard_proposal_as(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        actor: CandidateActor,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        if candidate.state == CandidateState::Discarded {
            return Ok(());
        }
        if !matches!(
            candidate.state,
            CandidateState::Ready | CandidateState::Partial | CandidateState::Interrupted
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        candidate.state = CandidateState::Discarded;
        candidate.reviewed_by = actor.user_id();
        candidate.updated_at = self.clock.now();
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Discard,
            actor,
            candidate.updated_at,
        ));
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}
