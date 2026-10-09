//! Same-proposal recovery, input freeze and worker fencing under the project transaction.
use super::{snapshot, validation::load};
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::{KnowledgeStore, KnowledgeTx};
use serde_json::Value;

pub(super) const LEASE_SECONDS: i64 = 120;
pub(super) fn owned(candidate: &Candidate, owner: Option<uuid::Uuid>) -> KnowledgeResult<()> {
    let expected = candidate
        .progress
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<uuid::Uuid>().ok());
    if expected != owner {
        return Err(KnowledgeError::CandidateClosed);
    }
    Ok(())
}
pub(super) async fn available(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    now: chrono::DateTime<chrono::Utc>,
) -> KnowledgeResult<()> {
    while let Some(active) = tx.active_candidate(project).await.map_err(store_error)? {
        if now - active.updated_at < chrono::TimeDelta::seconds(LEASE_SECONDS) {
            return Err(KnowledgeError::CandidateBusy);
        }
        let mut candidate = load(tx, project, active.id).await?;
        candidate.state = CandidateState::Interrupted;
        candidate.updated_at = now;
        tx.put_candidate(&candidate).await.map_err(store_error)?;
    }
    Ok(())
}
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn resume_proposal(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: Value,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        available(tx.as_mut(), project, self.clock.now()).await?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        if !matches!(
            candidate.state,
            CandidateState::Partial | CandidateState::Interrupted
        ) {
            return Err(KnowledgeError::CandidateClosed);
        }
        let endpoints = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        let base = snapshot::capture(tx.as_mut(), project, endpoints).await?;
        if snapshot::hash(&base) != candidate.base_hash {
            return Err(KnowledgeError::CandidateStale);
        }
        if candidate
            .execution_input
            .as_ref()
            .is_some_and(|frozen| frozen != &input)
        {
            return Err(KnowledgeError::CandidateStale);
        }
        candidate.execution_input = Some(input);
        // Close old batches: a late result from an obsolete worker cannot append to them.
        for batch in &mut candidate.batches {
            if batch.state == RoundState::Running {
                batch.state = RoundState::Failed;
            }
        }
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Resume,
            actor,
            self.clock.now(),
        ));
        candidate.state = CandidateState::Running;
        candidate.progress["run_id"] = serde_json::json!(owner);
        candidate.updated_at = self.clock.now();
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(candidate)
    }
    pub(super) async fn freeze_input(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        input: Value,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        super::running(&candidate)?;
        owned(&candidate, Some(owner))?;
        if candidate
            .execution_input
            .as_ref()
            .is_some_and(|old| old != &input)
        {
            return Err(KnowledgeError::CandidateStale);
        }
        candidate.execution_input = Some(input);
        candidate.updated_at = self.clock.now();
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    pub(super) async fn cancel_proposal(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = load(tx.as_mut(), project, id).await?;
        super::running(&candidate)?;
        candidate.state = CandidateState::Interrupted;
        candidate.progress["status"] = serde_json::json!("failed");
        candidate.progress["message"] =
            serde_json::json!("用户已停止本轮；已保存批次保留，可补齐本轮缺口");
        candidate.progress["active"] = serde_json::json!([]);
        if let Some(history) = candidate
            .progress
            .get_mut("attempt_history")
            .and_then(Value::as_array_mut)
            && let Some(attempt) = history.last_mut()
        {
            attempt["ended_at"] = serde_json::json!(self.clock.now());
            attempt["outcome"] = serde_json::json!("cancelled");
        }
        candidate.updated_at = self.clock.now();
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
    pub(super) async fn heartbeat(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        if tx.candidate_state(project, id).await.map_err(store_error)?
            != Some(CandidateState::Running)
            || tx.candidate_owner(project, id).await.map_err(store_error)? != Some(owner)
        {
            return Err(KnowledgeError::CandidateClosed);
        }
        tx.heartbeat_candidate(project, id, self.clock.now())
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}
