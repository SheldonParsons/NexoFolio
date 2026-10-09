//! Header provenance remains relevant even when its value is too long for a model input.
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldFacts, FieldLocation},
    knowledge::{EvidenceKind, EvidenceSource, FieldEvidence},
};
pub(crate) fn header_evidence(facts: &EndpointFacts, field: &FieldFacts) -> Vec<FieldEvidence> {
    let name = field.path.to_string();
    let mut seen = std::collections::HashSet::new();
    facts.requests.iter().filter_map(|request| {
        let headers=match field.location {
            FieldLocation::RequestHeader=>request.request.headers.as_ref(),
            FieldLocation::ResponseHeader {status} if request.response_status==Some(status)=>request.response_headers.as_ref(),
            _=>None,
        }?;
        let values:Vec<_>=headers.entries.iter().filter(|(key,_)|key.eq_ignore_ascii_case(&name)).map(|(_,value)|value.as_str()).collect();
        if values.is_empty() || !seen.insert((request.environment_id,values)) {return None;}
        Some(FieldEvidence {
            kind:EvidenceKind::Observation,
            text:"真实 Header 使用观测；完整值及重复项保存在请求资料中，出现不等于必填，历史凭据不代表当前有效".into(),
            endpoint:facts.summary.id,field:name.clone(),example:Some(request.example_id.clone()),environment:request.environment_id,options:vec![],
            source:Some(EvidenceSource {example:request.example_id.clone(),endpoint:Some(facts.summary.id),environment:request.environment_id,platform:Some(request.source.platform.clone()),location:Some(field.location),pointer:Some(format!("headers:{name}")),complete:Some(headers.complete),..Default::default()}),
        })
    }).take(12).collect()
}
