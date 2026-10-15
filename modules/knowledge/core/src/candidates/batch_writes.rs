//! One append rule path for full candidate callers and worker sessions.
use super::*;
use nexofolio_knowledge_contracts::KnowledgeTx;
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn append_proposal(
        &self,
        tx: &mut dyn KnowledgeTx,
        project: ProjectId,
        id: uuid::Uuid,
        owner: Option<uuid::Uuid>,
        batch: RoundId,
        proposed: &[Command],
    ) -> KnowledgeResult<KnowledgeSnapshot> {
        tx.lock_project(project).await.map_err(store_error)?;
        let mut context = tx
            .candidate_append(project, id, batch)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)?;
        validation::running_state(context.state)?;
        recovery::owned_by(context.owner, owner)?;
        super::tasks::worker(tx, project, id, owner, self.clock.now()).await?;
        let [group] = context.rounds.as_slice() else {
            return Err(KnowledgeError::RoundClosed);
        };
        if group.id != batch || group.state != RoundState::Running {
            return Err(KnowledgeError::RoundClosed);
        }
        let additions = CandidateBatch {
            id: batch,
            author: group.author.clone(),
            state: RoundState::Running,
            commands: proposed.to_vec(),
        };
        let mut preview = SnapshotTx {
            project,
            view: std::mem::take(&mut context.view),
        };
        commands(
            &mut preview,
            project,
            &context.endpoints,
            &additions,
            self.clock.now(),
            0,
        )
        .await?;
        valid_names(&preview.view)?;
        context.view = preview.view;
        tx.append_candidate_commands(&context, group, proposed, self.clock.now())
            .await
            .map_err(store_error)?;
        Ok(context.view)
    }
}
