//! Ephemeral evidence projection only; captured observations are never rewritten.
use nexofolio_common::EndpointId;
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldLocation, FieldPath, PathSegment},
    knowledge::DocumentLifecycle,
    observation::{Body, CanonicalObservation, Content, Fact, Headers, HttpRequest},
};
use serde_json::Value;

fn remove(value: &mut Value, path: &[PathSegment]) {
    match path.split_first() {
        None => *value = Value::Null,
        Some((PathSegment::Key(key), tail)) => {
            if tail.is_empty() {
                if let Some(o) = value.as_object_mut() {
                    o.remove(key);
                }
            } else if let Some(v) = value.get_mut(key) {
                remove(v, tail);
            }
        }
        Some((PathSegment::Items, tail)) => {
            if let Some(items) = value.as_array_mut() {
                for v in items {
                    remove(v, tail);
                }
            }
        }
    }
}
fn decode(value: &str) -> String {
    let mut bytes = Vec::new();
    let mut iter = value.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            if let (Some(a), Some(c)) = (iter.next(), iter.next())
                && let (Some(a), Some(c)) = ((a as char).to_digit(16), (c as char).to_digit(16))
            {
                bytes.push((a * 16 + c) as u8);
            }
        } else {
            bytes.push(if b == b'+' { b' ' } else { b });
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
fn encoded_parameters(text: &str, paths: &[FieldPath]) -> String {
    text.split('&')
        .filter(|part| {
            let key = decode(part.split('=').next().unwrap_or(""));
            !paths.iter().any(|p| {
                p.0.is_empty() || matches!(p.0.first(), Some(PathSegment::Key(k)) if *k == key)
            })
        })
        .collect::<Vec<_>>()
        .join("&")
}
fn paths(
    state: &DocumentLifecycle,
    endpoint: EndpointId,
    location: FieldLocation,
) -> Vec<FieldPath> {
    state
        .decisions
        .iter()
        .filter_map(|d| {
            if let nexofolio_contracts::knowledge::LifecycleTarget::Field {
                endpoint: id,
                location: at,
                path,
            } = &d.target
                && d.retired
                && *at == location
                && (*id == endpoint
                    || state
                        .aliases
                        .iter()
                        .any(|(old, current)| *old == *id && *current == endpoint))
            {
                return Some(path.clone());
            }
            None
        })
        .collect()
}
fn mark_projection(body: &mut Body) {
    const NOTE: &str = "生命周期有效投影：已排除废弃字段，原始报文保持不变";
    match body {
        Body::Full {
            media_type,
            content,
        } => {
            *body = Body::Partial {
                media_type: media_type.clone(),
                content: content.clone(),
                note: Some(NOTE.into()),
            };
        }
        Body::Partial { note, .. } if note.as_deref().is_none_or(|s| !s.contains(NOTE)) => {
            *note = Some(
                note.as_ref()
                    .map_or_else(|| NOTE.into(), |old| format!("{old}; {NOTE}")),
            );
        }
        _ => {} // A truncated original remains truncated, never upgraded to complete.
    }
}
fn body(body: &mut Body, paths: &[FieldPath]) {
    if paths.is_empty() {
        return;
    }
    let (media, content) = match body {
        Body::Full {
            media_type,
            content,
        }
        | Body::Partial {
            media_type,
            content,
            ..
        }
        | Body::Truncated {
            media_type,
            content,
            ..
        } => (media_type, content),
        _ => return,
    };
    if paths.iter().any(|p| p.0.is_empty()) {
        *body = Body::Unreadable {
            media_type: media.clone(),
            note: Some("body retired for curation".into()),
        };
        return;
    }
    if let Content::Text(text) = content {
        if media
            .as_deref()
            .is_some_and(|m| m.starts_with("application/x-www-form-urlencoded"))
        {
            *text = encoded_parameters(text, paths);
            mark_projection(body);
            return;
        }
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            if media
                .as_deref()
                .is_some_and(|m| m.starts_with("multipart/form-data"))
            {
                if let Some(entries) = value.as_array_mut() {
                    entries.retain(|v| !paths.iter().any(|p| matches!(p.0.first(), Some(PathSegment::Key(k)) if v["name"].as_str() == Some(k.as_str()))));
                }
            } else {
                for path in paths {
                    remove(&mut value, &path.0);
                }
            }
            *text = value.to_string();
            mark_projection(body);
            return;
        }
    }
    *body = Body::Unreadable {
        media_type: media.clone(),
        note: Some("retired fields cannot be separated from this representation".into()),
    };
}
fn headers(headers: &mut Option<Headers>, paths: &[FieldPath]) {
    if let Some(h) = headers {
        let before = h.entries.len();
        h.entries.retain(|(name,_)| !paths.iter().any(|p| p.0.is_empty() || matches!(p.0.first(),Some(PathSegment::Key(k)) if k.eq_ignore_ascii_case(name))));
        h.complete &= h.entries.len() == before;
    }
}
fn project_path(url: &str, template: &str, paths: &[FieldPath]) -> String {
    if paths.is_empty() {
        return url.into();
    }
    let split = url.find(['?', '#']).unwrap_or(url.len());
    let (address, suffix) = url.split_at(split);
    let mut actual: Vec<_> = address.split('/').collect();
    let template: Vec<_> = template.split('/').collect();
    for (back, part) in template.iter().rev().enumerate() {
        if let Some(name) = part.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
            && paths.iter().any(|p| {
                p.0.is_empty() || matches!(p.0.first(), Some(PathSegment::Key(key)) if key == name)
            })
            && back < actual.len()
        {
            let index = actual.len() - 1 - back;
            actual[index] = "";
        }
    }
    format!("{}{suffix}", actual.join("/"))
}
fn request(
    request: &mut HttpRequest,
    state: &DocumentLifecycle,
    endpoint: EndpointId,
    template: &str,
) {
    request.url = project_path(
        &request.url,
        template,
        &paths(state, endpoint, FieldLocation::Path),
    );
    body(
        &mut request.body,
        &paths(state, endpoint, FieldLocation::RequestBody),
    );
    headers(
        &mut request.headers,
        &paths(state, endpoint, FieldLocation::RequestHeader),
    );
    let query_paths = paths(state, endpoint, FieldLocation::Query);
    if !query_paths.is_empty()
        && let Some((base, query)) = request.url.split_once('?')
    {
        request.url = format!("{base}?{}", encoded_parameters(query, &query_paths));
    }
}
pub fn active_observation(
    raw: Value,
    state: &DocumentLifecycle,
    endpoint: EndpointId,
    template: &str,
) -> Value {
    let Ok(mut observation) = serde_json::from_value::<CanonicalObservation>(raw.clone()) else {
        return Value::Null;
    };
    if let Fact::Exchange(exchange) = &mut observation.fact {
        request(&mut exchange.request, state, endpoint, template);
        if let Some(response) = &mut exchange.response {
            body(
                &mut response.body,
                &paths(
                    state,
                    endpoint,
                    FieldLocation::ResponseBody {
                        status: response.status,
                    },
                ),
            );
            headers(
                &mut response.headers,
                &paths(
                    state,
                    endpoint,
                    FieldLocation::ResponseHeader {
                        status: response.status,
                    },
                ),
            );
        }
    }
    serde_json::to_value(observation).expect("observation")
}
pub fn active_facts(facts: &mut EndpointFacts, state: &DocumentLifecycle) {
    let id = facts.summary.id;
    facts
        .fields
        .retain(|f| state.field(id, f.location, &f.path).is_none());
    for material in &mut facts.requests {
        request(
            &mut material.request,
            state,
            id,
            &facts.summary.path_template,
        );
        if let Some(root) = &mut material.body_root {
            for path in paths(state, id, FieldLocation::RequestBody) {
                remove(root, &path.0);
            }
        }
        if let Some(status) = material.response_status {
            headers(
                &mut material.response_headers,
                &paths(state, id, FieldLocation::ResponseHeader { status }),
            );
        }
        material.authentication.retain(|a| !paths(state,id,FieldLocation::RequestHeader).iter().any(|p| matches!(p.0.first(),Some(PathSegment::Key(k)) if k.eq_ignore_ascii_case(&a.carrier))));
    }
}
