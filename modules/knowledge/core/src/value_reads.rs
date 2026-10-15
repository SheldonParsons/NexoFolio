//! Paged views of immutable observations. No model calls and no raw payload logging.
use crate::{Clock, Knowledge, store_error};
use nexofolio_contracts::{endpoint::EndpointReader, field_values::*, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::Value;
use std::collections::BTreeMap;
impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    async fn load_values(
        &self,
        q: &FieldValueQuery,
        value_id: Option<&str>,
    ) -> KnowledgeResult<FieldValueArchive> {
        if q.limit == 0 || q.limit > 50 || q.field.len() > 4096 {
            return Err(KnowledgeError::InvalidTree(
                "invalid value query bounds".into(),
            ));
        }
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let input = tx
            .candidate_input(q.project, q.archive)
            .await
            .map_err(store_error)?;
        if let Some(input) = input.filter(|v| v["version"] == 3) {
            let manifest: FrozenManifest =
                serde_json::from_value(input["manifest"].clone()).map_err(|_| frozen_invalid())?;
            if input["manifest_hash"].as_str() != Some(frozen_input_hash(&input).as_str())
                || manifest.version != 1
                || manifest.project != q.project
                || manifest.candidate != q.archive
            {
                return Err(frozen_invalid());
            }
            crate::candidates::frozen::read_value_selection(
                tx.as_mut(),
                &manifest,
                q.endpoint,
                Some(&q.field),
                value_id,
            )
            .await
        } else {
            tx.value_archive_field(q, value_id)
                .await
                .map_err(store_error)?
                .ok_or(KnowledgeError::NoSuchCandidate)
        }
    }
    pub(super) async fn read_value_page(
        &self,
        q: FieldValueQuery,
    ) -> KnowledgeResult<ObservedValuePage> {
        let archive = self.load_values(&q, None).await?;
        let field = archive
            .fields
            .get(&format!("{}/{}", q.endpoint, q.field))
            .ok_or(KnowledgeError::WrongProject)?;
        let mut states = BTreeMap::new();
        for c in field.coverage.iter().filter(|c| {
            q.environment
                .is_none_or(|e| c.source.environment == Some(e))
        }) {
            *states.entry(c.state.clone()).or_insert(0) += 1;
        }
        let entries: Vec<_> = field
            .values
            .iter()
            .filter(|entry| {
                entry.occurrences.iter().any(|p| {
                    archive.documents.get(&p.document).is_some_and(|d| {
                        q.environment
                            .is_none_or(|e| d.source.environment == Some(e))
                    })
                })
            })
            .collect();
        let summary = FieldObservationSummary {
            archive_id: Some(q.archive),
            total: entries.len(),
            hash: field.hash.clone(),
            states,
        };
        let items = entries
            .iter()
            .copied()
            .skip(q.offset)
            .take(q.limit)
            .map(|entry| {
                let value = value(&archive, entry)?;
                let encoded =
                    serde_json::to_string(value).map_err(|_| KnowledgeError::Unavailable)?;
                let scoped: Vec<_> = entry
                    .occurrences
                    .iter()
                    .filter(|p| {
                        archive.documents.get(&p.document).is_some_and(|d| {
                            q.environment
                                .is_none_or(|e| d.source.environment == Some(e))
                        })
                    })
                    .collect();
                let sources = scoped
                    .iter()
                    .copied()
                    .take(4)
                    .filter_map(|p| {
                        archive
                            .documents
                            .get(&p.document)
                            .map(|d| ObservedValueSource {
                                source: EvidenceSource {
                                    pointer: Some(p.pointer.clone()),
                                    ..d.source.clone()
                                },
                                observed_at: d.observed_at,
                                observation_key: d.observation_key,
                            })
                    })
                    .collect();
                Ok(ObservedValuePreview {
                    id: entry.id.clone(),
                    value_type: entry.value_type,
                    representation: entry.representation.clone(),
                    preview: encoded.chars().take(160).collect(),
                    preview_truncated: encoded.chars().count() > 160,
                    occurrences: scoped.len(),
                    sources,
                    sources_total: scoped.len(),
                })
            })
            .collect::<KnowledgeResult<Vec<_>>>()?;
        let has_more = q.offset.saturating_add(items.len()) < summary.total;
        Ok(ObservedValuePage {
            summary,
            items,
            offset: q.offset,
            limit: q.limit,
            has_more,
        })
    }
    pub(super) async fn read_value(
        &self,
        q: FieldValueQuery,
        value_id: &str,
    ) -> KnowledgeResult<Value> {
        let archive = self.load_values(&q, Some(value_id)).await?;
        let field = archive
            .fields
            .get(&format!("{}/{}", q.endpoint, q.field))
            .ok_or(KnowledgeError::WrongProject)?;
        let entry = field
            .values
            .iter()
            .find(|v| {
                v.id == value_id
                    && v.occurrences.iter().any(|p| {
                        archive.documents.get(&p.document).is_some_and(|d| {
                            q.environment
                                .is_none_or(|e| d.source.environment == Some(e))
                        })
                    })
            })
            .ok_or(KnowledgeError::NoSuchCandidate)?;
        Ok(value(&archive, entry)?.clone())
    }
}
fn value<'a>(
    archive: &'a FieldValueArchive,
    entry: &ObservedValueRef,
) -> KnowledgeResult<&'a Value> {
    let pointer = entry
        .occurrences
        .first()
        .ok_or(KnowledgeError::Unavailable)?;
    archive
        .documents
        .get(&pointer.document)
        .and_then(|d| d.root.pointer(&pointer.pointer))
        .ok_or(KnowledgeError::Unavailable)
}
