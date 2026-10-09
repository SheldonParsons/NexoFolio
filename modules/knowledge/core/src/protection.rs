//! Generated proposals cannot overwrite or erase human knowledge, even after a snapshot.
use crate::store_error;
use nexofolio_common::ProjectId;
use nexofolio_contracts::knowledge::*;
use nexofolio_knowledge_contracts::KnowledgeTx;

/// True means an identical human-owned row should stay human-owned.
pub(crate) async fn skip(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    author: &Author,
    command: &Command,
) -> KnowledgeResult<bool> {
    if author.is_person() {
        return Ok(false);
    }
    match command {
        Command::PutEndpointNote {
            endpoint,
            name,
            purpose,
            context,
        } => {
            if let Some(old) = tx
                .endpoint_note(*endpoint)
                .await
                .map_err(store_error)?
                .filter(|n| n.author.is_person())
            {
                if name.as_ref().is_none_or(|s| old.name.as_ref() == Some(s))
                    && purpose
                        .as_ref()
                        .is_none_or(|s| old.purpose.as_ref() == Some(s))
                    && &old.context == context
                {
                    return Ok(true);
                }
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        Command::PutFieldNote {
            endpoint,
            path,
            text,
            values,
            context,
        } => {
            if let Some(old) = tx
                .field_note(*endpoint, path)
                .await
                .map_err(store_error)?
                .filter(|n| n.author.is_person())
            {
                if &old.text == text && &old.values == values && &old.context == context {
                    return Ok(true);
                }
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        Command::RemoveFieldNote { endpoint, path } => {
            if tx
                .field_note(*endpoint, path)
                .await
                .map_err(store_error)?
                .is_some_and(|n| n.author.is_person())
            {
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        Command::PutLink {
            from,
            from_field,
            to,
            to_field,
            relation,
            evidence,
            context,
        } => {
            let identity = LinkIdentity::new(
                *from,
                from_field.clone(),
                *to,
                to_field.clone(),
                *relation,
                context,
            );
            if let Some(old) = tx
                .find_link(&identity)
                .await
                .map_err(store_error)?
                .filter(|l| l.author.is_person())
            {
                if &old.evidence == evidence && &old.context == context {
                    return Ok(true);
                }
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        Command::RemoveLink { id } => {
            if tx
                .link(*id)
                .await
                .map_err(store_error)?
                .is_some_and(|l| l.author.is_person())
            {
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        Command::RemoveFolder { id } => {
            let folders = tx.folders(project).await.map_err(store_error)?;
            let mut subtree = vec![*id];
            let mut index = 0;
            while index < subtree.len() {
                let parent = subtree[index];
                subtree.extend(
                    folders
                        .iter()
                        .filter(|f| f.parent == Some(parent))
                        .map(|f| f.id),
                );
                index += 1;
            }
            if folders
                .iter()
                .any(|f| subtree.contains(&f.id) && f.author.as_ref().is_none_or(|a| a.is_person()))
                || tx
                    .placements(project)
                    .await
                    .map_err(store_error)?
                    .iter()
                    .any(|p| subtree.contains(&p.folder) && p.author.is_person())
            {
                return Err(KnowledgeError::ProtectedKnowledge);
            }
        }
        _ => {}
    }
    Ok(false)
}
