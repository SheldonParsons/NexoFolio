//! Sanitized stored records: values/sequence/array indices retained only for selected business fields.
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::{endpoint::*, knowledge::AuthenticationUse, observation::*};
use nexofolio_curate::{SampleField, authentication, sample_fields};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Record {
    method: String,
    path: String,
    observation: CanonicalObservation,
}
pub struct Fixture {
    pub facts: Vec<EndpointFacts>,
    pub signals: Vec<SampleField>,
    pub authentication: Vec<(EndpointId, Vec<AuthenticationUse>)>,
}
impl Fixture {
    pub fn endpoint(&self, suffix: &str) -> &EndpointFacts {
        self.facts
            .iter()
            .find(|f| f.summary.path_template.ends_with(suffix))
            .unwrap()
    }
    pub fn environment(&self) -> EnvironmentId {
        self.signals[0].environment
    }
    #[allow(dead_code)] // Shared by the isolated PostgreSQL integration target.
    pub fn project(&self) -> ProjectId {
        self.facts[0].summary.project_id
    }
}
pub fn fixture() -> Fixture {
    let records: Vec<Record> =
        serde_json::from_str(include_str!("../fixtures/stage1-observations.json")).unwrap();
    let mut fixture = Fixture {
        facts: vec![],
        signals: vec![],
        authentication: vec![],
    };
    for (index, record) in records.iter().enumerate() {
        let id = if let Some(f) = fixture
            .facts
            .iter()
            .find(|f| f.summary.method == record.method && f.summary.path_template == record.path)
        {
            f.summary.id
        } else {
            let id: EndpointId = format!("00000000-0000-0000-0000-{:012}", fixture.facts.len() + 1)
                .parse()
                .unwrap();
            fixture.facts.push(EndpointFacts {
                requests: vec![],
                summary: EndpointSummary {
                    id,
                    project_id: record.observation.project_id,
                    method: record.method.clone(),
                    path_template: record.path.clone(),
                    external: false,
                    declared: false,
                    environments: vec![EnvironmentUsage {
                        environment_id: record.observation.environment_id.unwrap(),
                        calls: 1,
                        first_seen: record.observation.observed_at,
                        last_seen: record.observation.observed_at,
                    }],
                },
                aliases: vec![],
                addresses: vec![],
                fields: vec![],
                examples: vec![],
            });
            id
        };
        let facts = fixture
            .facts
            .iter_mut()
            .find(|f| f.summary.id == id)
            .unwrap();
        let Fact::Exchange(exchange) = &record.observation.fact else {
            panic!()
        };
        for (location, body) in [
            (FieldLocation::RequestBody, &exchange.request.body),
            (
                FieldLocation::ResponseBody {
                    status: exchange.response.as_ref().unwrap().status,
                },
                &exchange.response.as_ref().unwrap().body,
            ),
        ] {
            if let Body::Full {
                content: Content::Text(text),
                ..
            } = body
                && let Ok(json) = serde_json::from_str::<Value>(text)
            {
                walk(facts, location, vec![], &json);
            }
        }
        for segment in record.path.split('/').filter(|s| s.starts_with('{')) {
            field(
                facts,
                FieldLocation::Path,
                vec![PathSegment::Key(segment.trim_matches(['{', '}']).into())],
                ValueType::String,
            );
        }
        if let Some((_, query)) = exchange.request.url.split_once('?') {
            for parameter in query.split('&') {
                field(
                    facts,
                    FieldLocation::Query,
                    vec![PathSegment::Key(
                        parameter.split('=').next().unwrap().into(),
                    )],
                    ValueType::String,
                );
            }
        }
        let example = format!("sanitized-record-{index}");
        let raw = serde_json::to_value(&record.observation).unwrap();
        fixture.signals.extend(
            sample_fields(facts, &example, &raw)
                .into_iter()
                .filter(|s| s.value != "[REDACTED]"),
        );
        fixture
            .authentication
            .push((id, authentication(&example, &raw)));
    }
    fixture
}
fn field(
    facts: &mut EndpointFacts,
    location: FieldLocation,
    path: Vec<PathSegment>,
    kind: ValueType,
) {
    if let Some(f) = facts
        .fields
        .iter_mut()
        .find(|f| f.location == location && f.path.0 == path)
    {
        if !f.types.contains(&kind) {
            f.types.push(kind);
        }
    } else {
        facts.fields.push(FieldFacts {
            location,
            path: FieldPath(path),
            types: vec![kind],
            labels: vec![],
            differs_between_environments: false,
            declared: None,
            conflict: None,
        });
    }
}
fn walk(facts: &mut EndpointFacts, location: FieldLocation, path: Vec<PathSegment>, json: &Value) {
    let kind = match json {
        Value::Null => ValueType::Null,
        Value::Bool(_) => ValueType::Boolean,
        Value::Number(_) => ValueType::Number,
        Value::String(_) => ValueType::String,
        Value::Object(_) => ValueType::Object,
        Value::Array(_) => ValueType::Array,
    };
    field(facts, location, path.clone(), kind);
    match json {
        Value::Object(object) => {
            for (key, value) in object {
                let mut path = path.clone();
                path.push(PathSegment::Key(key.clone()));
                walk(facts, location, path, value);
            }
        }
        Value::Array(items) => {
            for item in items {
                let mut path = path.clone();
                path.push(PathSegment::Items);
                walk(facts, location, path, item);
            }
        }
        _ => {}
    }
}
