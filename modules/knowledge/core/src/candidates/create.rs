use super::{inheritance, recovery, snapshot, validation::load};
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;

impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn create_proposal_as(
        &self,
        project: ProjectId,
        target: Option<EndpointId>,
        parent: Option<uuid::Uuid>,
        actor: CandidateActor,
    ) -> KnowledgeResult<Candidate> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        recovery::available(tx.as_mut(), project, self.clock.now()).await?;
        let endpoints: Vec<_> = self
            .endpoints
            .list(project)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
            .into_iter()
            .map(|e| e.id)
            .collect();
        if target.is_some_and(|id| !endpoints.contains(&id)) {
            return Err(KnowledgeError::WrongProject);
        }
        let base = snapshot::capture(tx.as_mut(), project, endpoints).await?;
        let base_hash = snapshot::hash(&base);
        let mut candidate = Candidate {
            id: uuid::Uuid::new_v4(),
            project_id: project,
            target,
            parent: None,
            state: CandidateState::Running,
            created_at: self.clock.now(),
            updated_at: self.clock.now(),
            base_hash,
            view: base.clone(),
            base,
            batches: vec![],
            progress: serde_json::json!({}),
            execution_input: None,
            applied_rounds: vec![],
            reviewed_by: None,
            operations: Vec::new(),
        };
        candidate.view.lifecycle =
            crate::lifecycle::resolve(&self.endpoints, candidate.base.lifecycle.clone()).await?;
        let lifecycle = candidate.view.lifecycle.clone();
        if target.is_some_and(|id| lifecycle.endpoint(id).is_some()) {
            return Err(KnowledgeError::InvalidLifecycle("请先撤销接口废弃".into()));
        }
        lifecycle.project_snapshot(&mut candidate.view);
        if lifecycle.pending() {
            // Invalidating hashes only in the candidate preserves official history
            // and manual prose. New generated batches acquire fresh input hashes.
            for note in &mut candidate.view.notes {
                if !note.author.is_person() {
                    note.context.input_hash = None;
                }
            }
            for field in &mut candidate.view.fields {
                if !field.author.is_person() {
                    field.context.input_hash = None;
                }
            }
        }
        candidate.operations.push(CandidateOperation::new(
            CandidateAction::Organize,
            actor,
            candidate.created_at,
        ));
        if let Some(parent) = parent {
            let old = load(tx.as_mut(), project, parent).await?;
            if !matches!(
                old.state,
                CandidateState::Ready | CandidateState::Partial | CandidateState::Interrupted
            ) || old.base.lifecycle.version != candidate.base.lifecycle.version
                || (old.base_hash != candidate.base_hash
                    && !inheritance::compatible_base(&old.base, &candidate.base))
                || old.target != target
            {
                return Err(KnowledgeError::CandidateStale);
            }
            candidate.parent = Some(parent);
            candidate.view = old.view;
            candidate.view.endpoints = candidate.base.endpoints.clone();
            lifecycle.project_snapshot(&mut candidate.view);
            // A resumed proposal keeps every acknowledged chunk, including interrupted rounds.
            candidate.batches = old
                .batches
                .into_iter()
                .filter(|b| !b.commands.is_empty())
                .map(|mut b| {
                    b.state = RoundState::Done;
                    b
                })
                .collect();
        }
        tx.put_candidate(&candidate).await.map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(candidate)
    }
}
