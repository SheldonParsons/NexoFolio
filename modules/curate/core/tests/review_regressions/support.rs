use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::*;
pub fn facts(path: &str) -> EndpointFacts {
    EndpointFacts {
        requests: vec![],
        summary: EndpointSummary {
            id: EndpointId::new(),
            project_id: ProjectId::new(),
            method: "POST".into(),
            path_template: path.into(),
            external: false,
            declared: false,
            environments: vec![],
        },
        aliases: vec![],
        addresses: vec![],
        fields: vec![],
        examples: vec![],
    }
}
#[allow(clippy::too_many_arguments)]
pub fn sig(
    ep: EndpointId,
    env: EnvironmentId,
    name: &str,
    path: &str,
    value: &str,
    location: FieldLocation,
    seq: u64,
    row: &str,
) -> SampleField {
    SampleField {
        endpoint: ep,
        environment: env,
        name: name.into(),
        path: path.into(),
        value: value.into(),
        location,
        example: format!("sample-{seq}"),
        row: row.into(),
        source: EvidenceSource {
            endpoint: Some(ep),
            environment: Some(env),
            page: Some("same-page".into()),
            sequence: Some(seq),
            ..Default::default()
        },
    }
}
pub fn field(name: &str) -> FieldFacts {
    FieldFacts {
        location: FieldLocation::RequestBody,
        path: FieldPath(vec![PathSegment::Key(name.into())]),
        types: vec![ValueType::String],
        labels: vec![],
        differs_between_environments: false,
        declared: None,
        conflict: None,
    }
}
