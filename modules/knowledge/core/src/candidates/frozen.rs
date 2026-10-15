//! Chunk ownership, immutable publication and verified reads share the candidate transaction.
use crate::{Clock, Knowledge, store_error};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::{KnowledgeStore, KnowledgeTx};

pub(crate) async fn read_values(
    tx: &mut dyn KnowledgeTx,
    manifest: &FrozenManifest,
    endpoint: nexofolio_common::EndpointId,
) -> KnowledgeResult<nexofolio_contracts::field_values::FieldValueArchive> {
    read_value_selection(tx, manifest, endpoint, None, None).await
}

pub(crate) async fn read_value_selection(
    tx: &mut dyn KnowledgeTx,
    manifest: &FrozenManifest,
    endpoint: nexofolio_common::EndpointId,
    field: Option<&str>,
    value_id: Option<&str>,
) -> KnowledgeResult<nexofolio_contracts::field_values::FieldValueArchive> {
    let refs = manifest
        .endpoints
        .get(&endpoint)
        .ok_or(KnowledgeError::WrongProject)?;
    let mut archive: nexofolio_contracts::field_values::FieldValueArchive = refs
        .values
        .decode(&read_chunk(tx, manifest, &refs.values).await?)?;
    if archive.version != 1 || !archive.documents.is_empty() {
        return Err(frozen_invalid());
    }
    if let Some(field) = field {
        archive
            .fields
            .retain(|key, _| key == &format!("{endpoint}/{field}"));
        if let Some(id) = value_id {
            for field in archive.fields.values_mut() {
                field.values.retain(|v| v.id == id);
            }
        }
    }
    let documents: std::collections::HashSet<_> = archive
        .fields
        .values()
        .flat_map(|f| &f.values)
        .flat_map(|v| &v.occurrences)
        .map(|p| p.document.clone())
        .collect();
    if field.is_some()
        && documents
            .iter()
            .any(|key| !refs.documents.contains_key(key))
    {
        return Err(frozen_invalid());
    }
    for (key, reference) in &refs.documents {
        if field.is_some() && !documents.contains(key) {
            continue;
        }
        archive.documents.insert(
            key.clone(),
            reference.decode(&read_chunk(tx, manifest, reference).await?)?,
        );
    }
    if archive
        .documents
        .values()
        .any(|d| d.source.endpoint != Some(endpoint))
        || archive.fields.iter().any(|(key, field)| {
            !key.starts_with(&format!("{endpoint}/"))
                || field.values.iter().flat_map(|v| &v.occurrences).any(|p| {
                    archive
                        .documents
                        .get(&p.document)
                        .is_none_or(|d| d.root.pointer(&p.pointer).is_none())
                })
        })
    {
        return Err(frozen_invalid());
    }
    Ok(archive)
}

pub(crate) async fn read_chunk(
    tx: &mut dyn KnowledgeTx,
    manifest: &FrozenManifest,
    reference: &FrozenRef,
) -> KnowledgeResult<String> {
    let content = tx
        .frozen_chunk(
            manifest.project,
            manifest.candidate,
            manifest.snapshot,
            &reference.sha256,
        )
        .await
        .map_err(store_error)?
        .ok_or_else(frozen_invalid)?;
    reference.verify(&content)?;
    Ok(content)
}
pub(super) async fn validate(
    tx: &mut dyn KnowledgeTx,
    candidate: &Candidate,
    input: &serde_json::Value,
) -> KnowledgeResult<()> {
    if input["version"] != 3 {
        return Ok(());
    }
    let manifest: FrozenManifest =
        serde_json::from_value(input["manifest"].clone()).map_err(|_| frozen_invalid())?;
    if input["manifest_hash"].as_str() != Some(frozen_input_hash(input).as_str())
        || manifest.version != 1
        || manifest.project != candidate.project_id
        || manifest.candidate != candidate.id
        || manifest
            .endpoints
            .keys()
            .any(|e| !candidate.base.endpoints.contains(e))
        || input["lifecycle_version"].as_u64() != Some(candidate.base.lifecycle.version)
    {
        return Err(frozen_invalid());
    }
    let summaries: Vec<nexofolio_contracts::endpoint::EndpointSummary> =
        serde_json::from_value(input["summaries"].clone()).map_err(|_| frozen_invalid())?;
    if candidate
        .base
        .endpoints
        .iter()
        .filter(|e| candidate.base.lifecycle.endpoint(**e).is_none())
        .count()
        != manifest.endpoints.len()
        || summaries
            .iter()
            .map(|s| s.id)
            .collect::<std::collections::HashSet<_>>()
            .len()
            != summaries.len()
        || summaries.len() != manifest.endpoints.len()
        || summaries
            .iter()
            .any(|s| s.project_id != manifest.project || !manifest.endpoints.contains_key(&s.id))
    {
        return Err(frozen_invalid());
    }
    // Value/document references are all verified by read_values below. Do not
    // fetch and SHA-256 the same retained evidence twice during application.
    let mut verified: std::collections::HashSet<_> = manifest
        .endpoints
        .values()
        .flat_map(|e| std::iter::once(&e.values).chain(e.documents.values()))
        .map(|r| (r.sha256.as_str(), r.bytes))
        .collect();
    for reference in manifest.references() {
        if verified.insert((&reference.sha256, reference.bytes)) {
            read_chunk(tx, &manifest, reference).await?;
        }
    }
    for endpoint in manifest.endpoints.keys() {
        read_values(tx, &manifest, *endpoint).await?;
    }
    Ok(())
}
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn write_frozen_chunk(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
        owner: uuid::Uuid,
        snapshot: uuid::Uuid,
        content: &str,
    ) -> KnowledgeResult<FrozenRef> {
        let mut tx = self.store.begin().await.map_err(store_error)?;
        tx.lock_project(project).await.map_err(store_error)?;
        if tx
            .candidate_state(project, candidate)
            .await
            .map_err(store_error)?
            != Some(CandidateState::Running)
            || tx
                .candidate_owner(project, candidate)
                .await
                .map_err(store_error)?
                != Some(owner)
        {
            return Err(KnowledgeError::CandidateClosed);
        }
        super::tasks::worker(
            tx.as_mut(),
            project,
            candidate,
            Some(owner),
            self.clock.now(),
        )
        .await?;
        if tx
            .candidate_input(project, candidate)
            .await
            .map_err(store_error)?
            .is_some()
        {
            return Err(frozen_invalid());
        }
        let reference = FrozenRef::of(content);
        if let Some(old) = tx
            .frozen_chunk(project, candidate, snapshot, &reference.sha256)
            .await
            .map_err(store_error)?
        {
            reference.verify(&old)?;
        } else {
            tx.put_frozen_chunk(project, candidate, snapshot, &reference.sha256, content)
                .await
                .map_err(store_error)?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(reference)
    }
    pub(super) async fn read_frozen_chunk(
        &self,
        project: ProjectId,
        candidate: uuid::Uuid,
        snapshot: uuid::Uuid,
        reference: &FrozenRef,
    ) -> KnowledgeResult<String> {
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let content = tx
            .frozen_chunk(project, candidate, snapshot, &reference.sha256)
            .await
            .map_err(store_error)?
            .ok_or_else(frozen_invalid)?;
        reference.verify(&content)?;
        Ok(content)
    }
}
