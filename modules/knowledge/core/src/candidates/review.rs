use crate::{Clock, Knowledge, store_error};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{endpoint::EndpointReader, knowledge::*};
use nexofolio_knowledge_contracts::KnowledgeStore;
use serde_json::{Value, json};

impl<S: KnowledgeStore, R: EndpointReader, C: Clock> Knowledge<S, R, C> {
    pub(super) async fn review_proposal(
        &self,
        project: ProjectId,
        id: uuid::Uuid,
        q: &CandidateReviewQuery,
    ) -> KnowledgeResult<Value> {
        if q.offset > i32::MAX as usize
            || q.section.as_deref().is_some_and(|s| {
                !["folders", "placements", "notes", "fields", "links"].contains(&s)
            })
        {
            return Err(KnowledgeError::CandidateClosed);
        }
        if matches!(q.part, ReviewPart::Changes | ReviewPart::Endpoint) && q.version.is_none() {
            return Err(KnowledgeError::CandidateStale);
        }
        let mut tx = self.store.begin_read().await.map_err(store_error)?;
        let mut body = tx
            .review_candidate(project, id, q)
            .await
            .map_err(store_error)?
            .ok_or(KnowledgeError::NoSuchCandidate)?;
        if body["version_mismatch"] == true {
            return Err(KnowledgeError::CandidateStale);
        }
        if matches!(q.part, ReviewPart::Changes) {
            for item in body["items"].as_array_mut().ok_or_else(frozen_invalid)? {
                for side in ["before", "after"] {
                    if item[side].is_null() {
                        continue;
                    }
                    item[side] =
                        canonical(q.section.as_deref().unwrap_or("notes"), item[side].take())?;
                }
            }
        }
        if matches!(q.part, ReviewPart::Endpoint) {
            let endpoint = q.endpoint.ok_or(KnowledgeError::WrongProject)?;
            if body["included"] != true {
                return Err(KnowledgeError::WrongProject);
            }
            let input = body
                .as_object_mut()
                .unwrap()
                .remove("input")
                .unwrap_or(Value::Null);
            if body["execution_released"] != true && input["version"] == 3 {
                let manifest: FrozenManifest = serde_json::from_value(input["manifest"].clone())
                    .map_err(|_| frozen_invalid())?;
                if manifest.project != project
                    || manifest.candidate != id
                    || input["manifest_hash"] != frozen_input_hash(&input)
                {
                    return Err(frozen_invalid());
                }
                let refs = manifest
                    .endpoints
                    .get(&endpoint)
                    .ok_or(KnowledgeError::WrongProject)?;
                let mut wrapper: Value = refs.facts.decode(
                    &super::frozen::read_chunk(tx.as_mut(), &manifest, &refs.facts).await?,
                )?;
                let mut facts = wrapper
                    .as_object_mut()
                    .and_then(|w| w.remove("facts"))
                    .ok_or_else(frozen_invalid)?;
                if facts["summary"]["id"] != endpoint.to_string()
                    || facts["summary"]["project_id"] != project.to_string()
                {
                    return Err(frozen_invalid());
                }
                let fields = facts["fields"].as_array_mut().ok_or_else(frozen_invalid)?;
                body["fields_total"] = json!(fields.len());
                *fields = fields.iter().skip(q.offset).take(100).cloned().collect();
                body["facts"] = facts;
                let mut inputs = Vec::new();
                let mut offset = 0;
                for block in &refs.inputs {
                    let end = offset + block.keys.len();
                    if end > q.offset && offset < q.offset.saturating_add(100) {
                        let values: Vec<Value> = block.content.decode(
                            &super::frozen::read_chunk(tx.as_mut(), &manifest, &block.content)
                                .await?,
                        )?;
                        if values.len() != block.keys.len() {
                            return Err(frozen_invalid());
                        }
                        inputs.extend(
                            values
                                .into_iter()
                                .skip(q.offset.saturating_sub(offset))
                                .take(100 - inputs.len()),
                        );
                    }
                    offset = end;
                }
                body["inputs"] = json!(inputs);
            }
            if !body["facts"].is_null() {
                let fields: Vec<nexofolio_contracts::endpoint::FieldFacts> =
                    serde_json::from_value(body["facts"]["fields"].clone())
                        .map_err(|_| frozen_invalid())?;
                let keys: Vec<_> = fields.iter().map(field_note_key).collect();
                let selected = tx
                    .review_field_notes(project, id, endpoint, &keys)
                    .await
                    .map_err(store_error)?;
                let notes = body["notes"].as_array_mut().ok_or_else(frozen_invalid)?;
                for note in selected {
                    notes.retain(|n| n["path"] != note.path);
                    notes.push(serde_json::to_value(note).map_err(|_| frozen_invalid())?);
                }
            }
            // Released execution facts are explicitly unavailable. Retained candidate
            // notes/contexts and evidence archives remain readable; never borrow live facts.
            body["facts_available"] = json!(!body["facts"].is_null());
        }
        Ok(body)
    }
}

fn canonical(section: &str, value: Value) -> KnowledgeResult<Value> {
    fn typed<T: serde::de::DeserializeOwned + serde::Serialize>(
        value: Value,
    ) -> KnowledgeResult<Value> {
        serde_json::to_value(serde_json::from_value::<T>(value).map_err(|_| frozen_invalid())?)
            .map_err(|_| frozen_invalid())
    }
    match section {
        "folders" => typed::<Folder>(value),
        "placements" => typed::<Placement>(value),
        "notes" => typed::<EndpointNote>(value),
        "fields" => typed::<FieldNote>(value),
        "links" => typed::<Link>(value),
        _ => Err(KnowledgeError::CandidateClosed),
    }
}
