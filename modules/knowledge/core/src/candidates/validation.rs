use crate::{apply, store_error, touched};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge_contracts::KnowledgeTx;

pub(super) async fn load(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    id: uuid::Uuid,
) -> KnowledgeResult<Candidate> {
    tx.candidate(project, id)
        .await
        .map_err(store_error)?
        .ok_or(KnowledgeError::NoSuchCandidate)
}
pub(super) async fn load_projection(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    id: uuid::Uuid,
) -> KnowledgeResult<Candidate> {
    tx.candidate_projection(project, id)
        .await
        .map_err(store_error)?
        .ok_or(KnowledgeError::NoSuchCandidate)
}
pub(super) fn running(candidate: &Candidate) -> KnowledgeResult<()> {
    if candidate.state != CandidateState::Running {
        return Err(KnowledgeError::CandidateClosed);
    }
    Ok(())
}
pub(super) async fn commands(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    owned: &[EndpointId],
    batch: &CandidateBatch,
    at: chrono::DateTime<chrono::Utc>,
    offset: i64,
) -> KnowledgeResult<()> {
    for (index, command) in batch.commands.iter().enumerate() {
        if touched(command).iter().any(|e| !owned.contains(e)) {
            return Err(KnowledgeError::WrongProject);
        }
        // Folder and link IDs are also project-scoped, including deletions.
        let folders = match command {
            Command::PutFolder { id, parent, .. } => vec![Some(*id), *parent],
            Command::RemoveFolder { id } | Command::Place { folder: id, .. } => vec![Some(*id)],
            _ => vec![],
        };
        for id in folders.into_iter().flatten() {
            if tx
                .folder(id)
                .await
                .map_err(store_error)?
                .is_some_and(|f| f.project_id != project)
            {
                return Err(KnowledgeError::WrongProject);
            }
        }
        if let Command::RemoveLink { id } = command
            && tx
                .link(*id)
                .await
                .map_err(store_error)?
                .is_some_and(|l| !owned.contains(&l.from) || !owned.contains(&l.to))
        {
            return Err(KnowledgeError::WrongProject);
        }
        apply::one(
            tx,
            project,
            batch.id,
            offset + index as i64,
            &batch.author,
            at,
            command,
        )
        .await?;
    }
    Ok(())
}
pub(super) fn valid_names(view: &KnowledgeSnapshot) -> KnowledgeResult<()> {
    let mut names = std::collections::HashSet::new();
    if view
        .folders
        .iter()
        .any(|f| !names.insert((f.parent, f.name.trim())))
    {
        return Err(KnowledgeError::InvalidTree(
            "duplicate sibling names".into(),
        ));
    }
    Ok(())
}
