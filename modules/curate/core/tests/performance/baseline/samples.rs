//! Only bounded scalar values are sent to curation. Headers and raw bodies
//! never enter prompts. Evidence retains the original sample reference.
use nexofolio_common::{EndpointId, EnvironmentId};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldLocation, PathSegment},
    observation::{Body, CanonicalObservation, Content, Fact},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleField {
    pub endpoint: EndpointId,
    pub location: FieldLocation,
    pub path: String,
    pub name: String,
    pub value: String,
    pub example: String,
    pub environment: EnvironmentId,
    pub row: String,
    #[serde(default)]
    pub source: nexofolio_contracts::knowledge::EvidenceSource,
}

fn body_json(body: &Body) -> Option<Value> {
    match body {
        Body::Full {
            content: Content::Text(text),
            ..
        }
        | Body::Truncated {
            content: Content::Text(text),
            ..
        } => serde_json::from_str(text).ok(),
        _ => None,
    }
}

fn values_at<'a>(
    value: &'a Value,
    path: &[PathSegment],
    pointer: &str,
    out: &mut Vec<(&'a Value, String)>,
) {
    if out.len() >= 4096 {
        return;
    }
    match path.split_first() {
        None => out.push((
            value,
            pointer
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("")
                .into(),
        )),
        Some((PathSegment::Key(key), rest)) => {
            if let Some(value) = value.get(key) {
                values_at(value, rest, &format!("{pointer}/{key}"), out);
            }
        }
        Some((PathSegment::Items, rest)) => {
            if let Some(items) = value.as_array() {
                for (index, item) in items.iter().take(4096).enumerate() {
                    values_at(item, rest, &format!("{pointer}/{index}"), out);
                }
            }
        }
    }
}

pub(crate) fn sensitive(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "authorization",
        "cookie",
        "phone",
        "mobile",
        "email",
        "identitycard",
    ]
    .iter()
    .any(|key| name.contains(key))
}

fn parameter_text(text: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut input = text.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = (input.next()? as char).to_digit(16)?;
                let low = (input.next()? as char).to_digit(16)?;
                (high * 16 + low) as u8
            }
            _ => byte,
        });
    }
    String::from_utf8(bytes).ok()
}

pub fn sample_fields(facts: &EndpointFacts, example: &str, raw: &Value) -> Vec<SampleField> {
    let Ok(observation) = serde_json::from_value::<CanonicalObservation>(raw.clone()) else {
        return vec![];
    };
    let Some(environment) = observation.environment_id else {
        return vec![];
    };
    let source = super::context::source(&observation, example);
    let Fact::Exchange(exchange) = observation.fact else {
        return vec![];
    };
    let request_body = body_json(&exchange.request.body);
    let response_body = exchange.response.as_ref().and_then(|r| body_json(&r.body));
    let query = exchange
        .request
        .url
        .split_once('?')
        .map(|(_, query)| query.split('#').next().unwrap_or(""));
    let actual_path = exchange
        .request
        .url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|index| &rest[index..]))
        .unwrap_or("/")
        .split(['?', '#'])
        .next()
        .unwrap_or("/");
    let mut out = Vec::new();
    for field in &facts.fields {
        let path = field.path.to_string();
        let name = field
            .path
            .0
            .iter()
            .rev()
            .find_map(|s| match s {
                PathSegment::Key(key) => Some(key.clone()),
                _ => None,
            })
            .unwrap_or_default();
        if sensitive(&path) {
            continue;
        }
        let mut values = Vec::new();
        let mut parameter = Vec::new();
        match field.location {
            FieldLocation::RequestHeader | FieldLocation::ResponseHeader { .. } => {}
            FieldLocation::RequestBody => {
                if let Some(value) = &request_body {
                    values_at(value, &field.path.0, "", &mut values);
                }
            }
            FieldLocation::ResponseBody { status } => {
                if exchange
                    .response
                    .as_ref()
                    .is_some_and(|r| r.status == status)
                    && let Some(value) = &response_body
                {
                    values_at(value, &field.path.0, "", &mut values);
                }
            }
            FieldLocation::Query => {
                if let Some(query) = query {
                    parameter.extend(query.split('&').filter_map(|pair| {
                        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                        (parameter_text(key)?.as_str() == name)
                            .then(|| parameter_text(value))
                            .flatten()
                            .map(Value::String)
                    }));
                }
            }
            FieldLocation::Path => {
                let actual: Vec<_> = actual_path.split('/').collect();
                let template: Vec<_> = facts.summary.path_template.split('/').collect();
                if actual.len() == template.len() {
                    for (index, part) in template.iter().enumerate() {
                        if *part == format!("{{{name}}}") {
                            parameter.push(Value::String(actual[index].into()));
                        }
                    }
                }
            }
        }
        values.extend(parameter.iter().map(|value| (value, String::new())));
        for (value, row) in values {
            if value.is_object() || value.is_array() || value.is_null() {
                continue;
            }
            let value = match value {
                Value::String(text) => text.clone(),
                _ => value.to_string(),
            };
            if value.is_empty() || value.chars().count() > 80 {
                continue;
            }
            if out.iter().any(|s: &SampleField| {
                s.location == field.location && s.path == path && s.value == value && s.row == row
            }) {
                continue;
            }
            out.push(SampleField {
                endpoint: facts.summary.id,
                location: field.location,
                path: path.clone(),
                name: name.clone(),
                value,
                example: example.into(),
                environment,
                source: nexofolio_contracts::knowledge::EvidenceSource {
                    endpoint: Some(facts.summary.id),
                    complete: Some(match field.location {
                        FieldLocation::RequestBody => exchange.request.body.proves_absence(),
                        FieldLocation::ResponseBody { .. } => exchange
                            .response
                            .as_ref()
                            .is_some_and(|r| r.body.proves_absence()),
                        _ => !exchange.request.url_truncated,
                    }),
                    location: Some(field.location),
                    pointer: Some(match field.location {
                        FieldLocation::Query => format!("request.url.query:{name}"),
                        FieldLocation::Path => format!("request.url.path_parameter:{name}"),
                        _ => format!("{row}/{name}"),
                    }),
                    ..source.clone()
                },
                row,
            });
        }
    }
    out
}
