//! Compatibility projection for callers that omit section.
use super::*;
impl Navigation {
    pub(super) async fn legacy_interface(&self, request: &Query) -> Result<Value, ReadError> {
        let endpoint = request.endpoint_id.ok_or("ENDPOINT_REQUIRED")?;
        let facts = self
            .endpoints
            .get(endpoint)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?
            .ok_or("NOT_FOUND")?;
        let knowledge = self
            .knowledge
            .endpoint(endpoint)
            .await
            .map_err(|_| "KNOWLEDGE_UNAVAILABLE")?;
        let fields: Vec<_> = facts.fields.iter().skip(request.field_offset).take(request.limit).map(|f| {
                    let key = nexofolio_contracts::knowledge::field_note_key(f);
                    let note = knowledge.fields.iter().find(|n| n.path == key || (n.context.location.is_none() && n.path == f.path.to_string()));
                    json!({"key":key,"path":f.path,"location":f.location,"types":f.types,"labels":f.labels,
                        "knowledge":note})
                }).collect();
        let links: Vec<_> = knowledge
            .links
            .iter()
            .skip(request.link_offset)
            .take(request.limit)
            .collect();
        let materials: Vec<_> = facts
            .requests
            .iter()
            .filter(|r| {
                request
                    .environment_id
                    .is_none_or(|e| r.environment_id == Some(e))
            })
            .collect();
        let material_page: Vec<_> = materials.iter().skip(request.offset).take(request.limit).map(|r| json!({
                    "example_id":r.example_id,"environment_id":r.environment_id,"observed_at":r.observed_at,
                    "body_state":serde_json::to_value(&r.request.body).ok().and_then(|b| b.get("state").cloned()),
                    "headers_complete":r.request.headers.as_ref().map(|h| h.complete),"gaps":r.gaps,
                    "read_with":"read_request_material"
                })).collect();
        Ok(
            json!({"facts":facts.summary,"addresses":facts.addresses,"note":knowledge.note,
                    "fields":fields,"field_total":facts.fields.len(),"field_offset":request.field_offset,
                    "fields_has_more":request.field_offset+request.limit<facts.fields.len(),
                    "links":links,"link_total":knowledge.links.len(),"link_offset":request.link_offset,
                    "links_has_more":request.link_offset+request.limit<knowledge.links.len(),
                    "request_materials":material_page,"request_material_total":materials.len(),
                    "request_material_has_more":request.offset+request.limit<materials.len(),
                    "request_material_offset":request.offset,"raw_samples_returned":false}),
        )
    }
}
