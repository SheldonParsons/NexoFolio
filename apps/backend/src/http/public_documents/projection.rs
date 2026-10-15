use super::types;
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::*, knowledge::*};

pub(super) fn endpoint(
    f: EndpointFacts,
    k: EndpointKnowledge,
    env: Option<EnvironmentId>,
) -> types::DocumentEndpoint {
    let note = k.note.as_ref();
    let fields = f
        .fields
        .into_iter()
        .filter(|f| {
            env.is_none_or(|e| {
                f.location == FieldLocation::Path
                    || f.labels.is_empty()
                    || f.labels
                        .iter()
                        .any(|l| l.environment_id == e && !matches!(l.label, FieldLabel::Absent))
            })
        })
        .map(|f| {
            let key = field_note_key(&f);
            let note = k.fields.iter().find(|n| n.path == key).or_else(|| {
                k.fields
                    .iter()
                    .find(|n| n.context.location.is_none() && n.path == f.path.to_string())
            });
            types::Field {
                retired_sources: false,
                key,
                structure: types::Structure {
                    location: f.location,
                    path: f.path,
                    types: f.types,
                    labels: f.labels,
                    differs_between_environments: f.differs_between_environments,
                    declared: f.declared,
                    conflict: f.conflict,
                },
                name: note.and_then(|n| n.context.display_name.clone()),
                description: note.map(|n| n.text.clone()),
                details: note
                    .map(|n| n.context.meaning.details.clone())
                    .unwrap_or_default(),
                unknowns: note
                    .map(|n| n.context.meaning.unknowns.clone())
                    .unwrap_or_default(),
                confirmation: note.map(|n| n.context.meaning.confirmation),
            }
        })
        .collect();
    types::DocumentEndpoint {
        retired_source_count: 0,
        project_id: f.summary.project_id,
        endpoint_id: f.summary.id,
        environment_id: env,
        method: f.summary.method,
        path: f.summary.path_template,
        name: note.and_then(|n| n.name.clone()),
        purpose: note.and_then(|n| n.purpose.clone()),
        description: note.and_then(|n| n.context.description.clone()),
        unknowns: note.map(|n| n.context.unknowns.clone()).unwrap_or_default(),
        fields,
    }
}
