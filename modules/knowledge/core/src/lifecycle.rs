//! Knowledge owns decisions; observe only resolves identities and validates facts.
use crate::{Clock, Knowledge, store_error};
use async_trait::async_trait;
use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::{KnowledgeStore, KnowledgeTx};

/// Called inside the project writer transaction, never from a read projection.
pub(crate) async fn validate_folder_names(
    tx: &mut dyn KnowledgeTx,
    project: ProjectId,
    state: &DocumentLifecycle,
) -> KnowledgeResult<()> {
    let folders = tx.folders(project).await.map_err(store_error)?;
    crate::tree::check_names(
        folders
            .iter()
            .filter(|f| state.folder(f.id).is_none())
            .map(|f| (f.parent, f.name.as_str())),
    )
}

pub(crate) async fn resolve(
    reader: &impl EndpointReader,
    mut state: DocumentLifecycle,
) -> KnowledgeResult<DocumentLifecycle> {
    state.aliases.clear();
    let mut visited = std::collections::HashSet::new();
    for d in &state.decisions {
        let id = match d.target {
            LifecycleTarget::Endpoint { id } => id,
            LifecycleTarget::Field { endpoint, .. } => endpoint,
            _ => continue,
        };
        if !visited.insert(id) {
            continue;
        }
        if let Some(facts) = reader
            .identity(id)
            .await
            .map_err(|_| KnowledgeError::Unavailable)?
        {
            if id != facts.id {
                state.aliases.push((id, facts.id));
            }
            for alias in facts.aliases {
                state.aliases.push((alias, facts.id));
            }
        }
    }
    state
        .aliases
        .sort_by_key(|(a, b)| (a.to_string(), b.to_string()));
    state.aliases.dedup();
    Ok(state)
}
#[async_trait]
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> DocumentLifecycleWriter
    for Knowledge<S, R, C>
{
    async fn change_lifecycle(
        &self,
        project: ProjectId,
        change: LifecycleChange,
        actor: CandidateActor,
    ) -> KnowledgeResult<DocumentLifecycle> {
        if actor.user_id().is_none() {
            return Err(KnowledgeError::InvalidLifecycle("需要登录用户".into()));
        }
        let reason = change.reason.trim();
        if (change.retired && reason.is_empty()) || reason.chars().count() > 1000 {
            return Err(KnowledgeError::InvalidLifecycle(
                "请填写不超过 1000 字的废弃原因".into(),
            ));
        }
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let mut state = tx.lifecycle(project).await.map_err(store_error)?;
        if state.version != change.expected_version {
            return Err(KnowledgeError::LifecycleStale);
        }
        let previous = state
            .decisions
            .iter()
            .find(|d| d.target == change.target)
            .cloned();
        // Undo remains available even if the field has disappeared from current traffic.
        if !change.retired && previous.as_ref().is_none_or(|d| !d.retired) {
            return Err(KnowledgeError::InvalidLifecycle(
                "该对象没有直接废弃决定，请管理所属接口、字段或目录".into(),
            ));
        }
        let mut folders = previous
            .as_ref()
            .map(|d| d.folders.clone())
            .unwrap_or_default();
        match &change.target {
            LifecycleTarget::Folder { id } => {
                let all = tx.folders(project).await.map_err(store_error)?;
                if !all.iter().any(|f| f.id == *id) {
                    return Err(KnowledgeError::NoSuchFolder);
                }
                if change.retired {
                    folders = vec![*id];
                    loop {
                        let next: Vec<_> = all
                            .iter()
                            .filter(|f| {
                                f.parent.is_some_and(|p| folders.contains(&p))
                                    && !folders.contains(&f.id)
                            })
                            .map(|f| f.id)
                            .collect();
                        if next.is_empty() {
                            break;
                        }
                        folders.extend(next);
                    }
                }
            }
            LifecycleTarget::Endpoint { id } | LifecycleTarget::Field { endpoint: id, .. } => {
                let facts = self
                    .endpoints
                    .get(*id)
                    .await
                    .map_err(|_| KnowledgeError::Unavailable)?
                    .ok_or(KnowledgeError::WrongProject)?;
                if facts.summary.project_id != project {
                    return Err(KnowledgeError::WrongProject);
                }
                if let LifecycleTarget::Field { location, path, .. } = &change.target
                    && change.retired
                    && !facts
                        .fields
                        .iter()
                        .any(|f| f.location == *location && f.path == *path)
                {
                    return Err(KnowledgeError::InvalidLifecycle("字段路径不存在".into()));
                }
            }
        }
        let effective = resolve(&self.endpoints, state.clone()).await?;
        if change.retired {
            let inherited = match &change.target {
                LifecycleTarget::Folder { id } => effective.folder(*id),
                LifecycleTarget::Endpoint { id } => effective.endpoint(*id),
                LifecycleTarget::Field {
                    endpoint,
                    location,
                    path,
                } => effective.field(*endpoint, *location, path),
            };
            if inherited.is_some() {
                return Err(KnowledgeError::InvalidLifecycle(
                    "对象已废弃，请管理原废弃决定".into(),
                ));
            }
        }
        state.version += 1;
        state.decisions.retain(|d| d.target != change.target);
        state.decisions.push(Retirement {
            target: change.target,
            retired: change.retired,
            reason: reason.into(),
            actor,
            at: self.clock.now(),
            version: state.version,
            folders,
        });
        if matches!(
            state.decisions.last().map(|d| &d.target),
            Some(LifecycleTarget::Folder { .. })
        ) {
            validate_folder_names(tx.as_mut(), project, &state)
                .await
                .map_err(|error| match error {
                    KnowledgeError::InvalidTree(message) => KnowledgeError::InvalidLifecycle(
                        format!("{message}；请先废弃或调整同名目录，再撤销旧目录的废弃"),
                    ),
                    other => other,
                })?;
        }
        state.aliases.clear();
        tx.put_lifecycle(project, &state)
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        resolve(&self.endpoints, state).await
    }
}
