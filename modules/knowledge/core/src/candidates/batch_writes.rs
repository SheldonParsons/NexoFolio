//! Shared validation for full candidate reads and the worker's input-free projection.
use super::*;
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn append_proposal(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        proposed: &[Command],
        projection_only: bool,
    ) -> KnowledgeResult<Candidate> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut candidate = if projection_only {
            validation::load_projection(tx.as_mut(), project, id).await?
        } else {
            load(tx.as_mut(), project, id).await?
        };
        running(&candidate)?;
        recovery::owned(&candidate, owner)?;
        let group = candidate
            .batches
            .iter()
            .find(|b| b.id == batch && b.state == RoundState::Running)
            .ok_or(KnowledgeError::RoundClosed)?;
        let additions = CandidateBatch {
            id: batch,
            author: group.author.clone(),
            state: RoundState::Running,
            commands: proposed.to_vec(),
        };
        let mut preview = SnapshotTx {
            project,
            view: candidate.view.clone(),
        };
        commands(
            &mut preview,
            project,
            &candidate.base.endpoints,
            &additions,
            self.clock.now(),
            0,
        )
        .await?;
        valid_names(&preview.view)?;
        candidate.view = preview.view;
        candidate
            .batches
            .iter_mut()
            .find(|b| b.id == batch)
            .unwrap()
            .commands
            .extend_from_slice(proposed);
        candidate.updated_at = self.clock.now();
        if projection_only {
            tx.put_candidate_projection(&candidate)
                .await
                .map_err(store_error)?;
        } else {
            tx.put_candidate(&candidate).await.map_err(store_error)?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(candidate)
    }
}
