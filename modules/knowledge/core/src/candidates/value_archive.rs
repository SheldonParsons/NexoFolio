//! Persist immutable factual input under the same ownership/baseline as the candidate.
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{
    endpoint::EndpointReader, field_values::FieldValueArchive, knowledge::*,
};
use nexofolio_knowledge_contracts::KnowledgeStore;
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn freeze_values(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        owner: uuid::Uuid,
        archive: &FieldValueArchive,
    ) -> KnowledgeResult<()> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        let candidate = super::load(tx.as_mut(), project, id).await?;
        super::running(&candidate)?;
        super::recovery::owned(&candidate, Some(owner))?;
        super::tasks::worker(tx.as_mut(), project, id, Some(owner), self.clock.now()).await?;
        if archive.version != 1
            || archive.documents.values().any(|d| {
                d.source
                    .endpoint
                    .is_none_or(|e| !candidate.base.endpoints.contains(&e))
            })
        {
            return Err(KnowledgeError::WrongProject);
        }
        for (key, field) in &archive.fields {
            let owned = candidate
                .base
                .endpoints
                .iter()
                .any(|id| key.starts_with(&format!("{id}/")));
            if !owned
                || field.values.iter().flat_map(|v| &v.occurrences).any(|p| {
                    archive
                        .documents
                        .get(&p.document)
                        .is_none_or(|d| d.root.pointer(&p.pointer).is_none())
                })
            {
                return Err(KnowledgeError::WrongProject);
            }
        }
        if let Some(existing) = tx.value_archive(project, id).await.map_err(store_error)? {
            if &existing != archive {
                return Err(KnowledgeError::CandidateStale);
            }
        } else {
            tx.put_value_archive(project, id, archive)
                .await
                .map_err(store_error)?;
        }
        tx.commit().await.map_err(store_error)
    }
}
