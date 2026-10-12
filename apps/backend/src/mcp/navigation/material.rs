//! Read retained HTTP/source documents. No live requests and no hidden redaction.
use super::projection::document;
use super::*;
use nexofolio_contracts::{
    observation::{CanonicalObservation, Fact},
    request::{RequestMaterial, body_root},
};
impl Navigation {
    pub(super) async fn material(
        &self,
        tool: &str,
        q: &Query,
        summaries: &[EndpointSummary],
    ) -> Result<Value, ReadError> {
        if tool == "read_source_evidence" && q.directory_id.is_some() {
            if q.endpoint_id.is_some()
                || q.example_id.is_some()
                || q.field_key.is_some()
                || q.link_id.is_some()
            {
                return Err("INVALID_ARGUMENTS".into());
            }
            let snapshot = self
                .knowledge
                .catalogue_navigation(q.project_id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
            let f = snapshot
                .catalogue
                .folders
                .iter()
                .find(|f| Some(f.id) == q.directory_id)
                .ok_or("DIRECTORY_NOT_IN_PROJECT")?;
            return document(
                tool,
                q,
                json!({"directory":f,"path":super::catalogue::breadcrumb(&snapshot.catalogue.folders,f.id),"knowledge_view":"official"}),
            );
        }
        let id = q.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
        if tool == "read_source_evidence" && q.example_id.is_none() {
            if q.field_key.is_some() && q.link_id.is_some() {
                return Err("INVALID_ARGUMENTS".into());
            }
            let k = self
                .knowledge
                .endpoint(id)
                .await
                .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
            let mut sources = Vec::new();
            let mut value = if let Some(key) = &q.field_key {
                let n = if let Some(n) = k.fields.iter().find(|n| n.path == *key) {
                    n.clone()
                } else {
                    let facts = self
                        .endpoints
                        .get(id)
                        .await
                        .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
                        .ok_or("NOT_FOUND")?;
                    let field = facts
                        .fields
                        .iter()
                        .find(|f| nexofolio_contracts::knowledge::field_note_key(f) == *key)
                        .ok_or("FIELD_NOT_FOUND")?;
                    super::interface::note_for(field, &k)
                        .cloned()
                        .ok_or("FIELD_NOT_FOUND")?
                };
                sources.extend(
                    n.context
                        .evidence
                        .iter()
                        .filter(|e| {
                            q.environment_id.is_none_or(|env| {
                                e.environment
                                    .or_else(|| e.source.as_ref().and_then(|s| s.environment))
                                    .is_none_or(|v| v == env)
                            })
                        })
                        .filter_map(super::source_actions::field_source),
                );
                json!({"endpoint_id":id,"field_key":key,"author":n.author,"meaning_confirmation":n.context.meaning.confirmation,
     "evidence":n.context.evidence.iter().filter(|e|q.environment_id.is_none_or(|env|e.environment.is_none_or(|v|v==env))).collect::<Vec<_>>()})
            } else if let Some(link) = q.link_id {
                let l = k
                    .links
                    .iter()
                    .find(|l| l.id == link)
                    .ok_or("RELATION_NOT_FOUND")?;
                if q.environment_id
                    .is_some_and(|e| l.context.environment.is_some_and(|v| v != e))
                {
                    return Err("ENVIRONMENT_MISMATCH".into());
                }
                sources.extend(l.context.sources.iter().cloned());
                serde_json::to_value(l).map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
            } else {
                if let Some(n) = &k.note {
                    sources.extend(n.context.sources.iter().cloned());
                }
                json!({"endpoint_id":id,"note":k.note,"directory_path":k.path})
            };
            value["source_reads"] = json!(super::source_actions::reads(q, &sources, summaries));
            value["source_read_note"] = json!(
                "Direct actions use explicit source references without prefetch. If no longer retained, inspect fallback_samples; another sample is not the original evidence."
            );
            return document(tool, q, value);
        }
        if q.field_key.is_some() || q.link_id.is_some() {
            return Err("INVALID_ARGUMENTS".into());
        }
        let example = q.example_id.as_deref().ok_or("EXAMPLE_REQUIRED")?;
        let raw = self
            .endpoints
            .example(id, example)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
            .ok_or("SAMPLE_NOT_RETAINED")?;
        let observation: CanonicalObservation =
            serde_json::from_value(raw).map_err(|_| "REQUEST_UNREADABLE")?;
        if observation.project_id != q.project_id {
            return Err("ENDPOINT_NOT_IN_PROJECT".into());
        }
        if q.environment_id
            .is_some_and(|e| observation.environment_id != Some(e))
        {
            return Err("ENVIRONMENT_MISMATCH".into());
        }
        if q.source_record.is_some_and(|s| s != observation.key) {
            return Err("SOURCE_CHANGED_RESTART_READ".into());
        }
        let mut value = match tool {
            "read_source_evidence" => {
                json!({"observation_key":observation.key,"source":observation.source,"observed_at":observation.observed_at,"environment_id":observation.environment_id,"site":observation.site,"context":observation.context,"context_state":if observation.context.is_some(){"retained"}else{"not_captured"}})
            }
            "read_request_material" => {
                let material = RequestMaterial::from_observation(example, &observation)
                    .ok_or("REQUEST_UNREADABLE")?;
                match q.part.as_deref().unwrap_or("all") {
                    "all" => serde_json::to_value(material).map_err(|_| "REQUEST_UNREADABLE")?,
                    "headers" => {
                        json!({"observation_key":observation.key,"headers":material.request.headers,"authentication":material.authentication,"gaps":material.gaps})
                    }
                    "url" => {
                        json!({"observation_key":observation.key,"method":material.request.method,"url":material.request.url,"url_truncated":material.request.url_truncated})
                    }
                    "body" => {
                        json!({"observation_key":observation.key,"body":material.request.body,"body_root":material.body_root,"body_root_kind":material.body_root_kind,"gaps":material.gaps})
                    }
                    _ => return Err("INVALID_ARGUMENTS".into()),
                }
            }
            "read_response_material" => {
                let Fact::Exchange(exchange) = &observation.fact else {
                    return Err("RESPONSE_UNREADABLE".into());
                };
                let mut v = json!({"observation_key":observation.key,"response_state":if exchange.response.is_some(){"captured"}else{"not_captured"}});
                if let Some(r) = &exchange.response {
                    v["status"] = json!(r.status);
                    match q.part.as_deref().unwrap_or("all") {
                        "all" => {
                            v["headers"] = json!(r.headers);
                            v["body"] = json!(r.body);
                            v["body_root"] = json!(body_root(&r.body));
                        }
                        "headers" => {
                            v["headers"] = json!(r.headers);
                        }
                        "body" => {
                            v["body"] = json!(r.body);
                            v["body_root"] = json!(body_root(&r.body));
                        }
                        _ => return Err("INVALID_ARGUMENTS".into()),
                    }
                }
                v
            }
            _ => return Err("UNKNOWN_TOOL".into()),
        };
        value["environment_id"] = json!(observation.environment_id);
        value["observed_at"] = json!(observation.observed_at);
        let mut out = document(tool, q, value)?;
        if let Some(next) = out["next"]["arguments"].as_object_mut() {
            next.insert("source_record".into(), json!(observation.key));
        }
        Ok(out)
    }
}
