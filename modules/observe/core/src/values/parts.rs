//! Decoding captured values never changes the captured body/header completeness.
use crate::structure::{essence, is_json_media};
use nexofolio_contracts::{
    endpoint::FieldLocation,
    observation::{Body, Content, Headers, HttpExchange},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) struct Part {
    pub root: Option<Value>,
    pub representation: String,
    pub state: String,
    pub pairs: bool,
    pub original: Option<Value>,
}
impl Part {
    fn unavailable(state: &str) -> Self {
        Self {
            root: None,
            representation: "unknown".into(),
            state: state.into(),
            pairs: false,
            original: None,
        }
    }
}
pub(super) fn part(
    exchange: &HttpExchange,
    location: FieldLocation,
    path_template: &str,
) -> Option<Part> {
    Some(match location {
        FieldLocation::RequestHeader => headers(exchange.request.headers.as_ref()),
        FieldLocation::ResponseHeader { status } => headers(
            exchange
                .response
                .as_ref()
                .filter(|r| r.status == status)?
                .headers
                .as_ref(),
        ),
        FieldLocation::RequestBody => body(&exchange.request.body),
        FieldLocation::ResponseBody { status } => body(
            &exchange
                .response
                .as_ref()
                .filter(|r| r.status == status)?
                .body,
        ),
        FieldLocation::Query => {
            let query = exchange
                .request
                .url
                .split_once('?')
                .map(|(_, v)| v.split('#').next().unwrap_or(""))
                .unwrap_or("");
            Part {
                root: Some(pairs(query)),
                representation: "query_parameters".into(),
                state: if exchange.request.url_truncated {
                    "truncated"
                } else {
                    "full"
                }
                .into(),
                pairs: true,
                original: None,
            }
        }
        FieldLocation::Path => {
            let path = crate::structure::path_of(&exchange.request.url);
            let actual: Vec<_> = path.split('/').collect();
            let template: Vec<_> = path_template.split('/').collect();
            let mut root = BTreeMap::new();
            if actual.len() != template.len() {
                return Some(Part::unavailable("unavailable"));
            }
            for (t, v) in template.iter().zip(actual) {
                if t.starts_with('{') && t.ends_with('}') {
                    root.insert(
                        t[1..t.len() - 1].to_owned(),
                        Value::String(decode(v, false)),
                    );
                }
            }
            Part {
                root: Some(json!(root)),
                representation: "path_parameters".into(),
                state: "full".into(),
                pairs: false,
                original: None,
            }
        }
    })
}
fn headers(headers: Option<&Headers>) -> Part {
    let Some(headers) = headers else {
        return Part::unavailable("unavailable");
    };
    Part {
        root: Some(json!(headers.entries)),
        representation: "headers".into(),
        state: if headers.complete { "full" } else { "partial" }.into(),
        pairs: true,
        original: None,
    }
}
fn body(body: &Body) -> Part {
    let (media, content, state) = match body {
        Body::None => return Part::unavailable("none"),
        Body::Unreadable { .. } => return Part::unavailable("unreadable"),
        Body::Full {
            media_type,
            content,
        } => (media_type, content, "full"),
        Body::Truncated {
            media_type,
            content,
            ..
        } => (media_type, content, "truncated"),
        Body::Partial {
            media_type,
            content,
            ..
        } => (media_type, content, "partial"),
    };
    let media = media.as_deref().map(essence);
    match content {
        Content::Binary(bytes) => Part {
            root: Some(json!(bytes)),
            representation: "binary".into(),
            state: state.into(),
            pairs: false,
            original: None,
        },
        Content::Text(text) => {
            if media.as_deref() == Some("application/x-www-form-urlencoded") {
                return Part {
                    root: Some(pairs(text)),
                    representation: "urlencoded_parameters".into(),
                    state: state.into(),
                    pairs: true,
                    original: Some(Value::String(text.clone())),
                };
            }
            if media.as_deref() == Some("multipart/form-data") {
                if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(text) {
                    return Part {
                        root: Some(Value::Array(entries)),
                        representation: "multipart_summary".into(),
                        state: "partial".into(),
                        pairs: false,
                        original: None,
                    };
                }
                return Part {
                    root: Some(Value::String(text.clone())),
                    representation: "multipart_raw".into(),
                    state: state.into(),
                    pairs: false,
                    original: None,
                };
            }
            match serde_json::from_str(text) {
                Ok(root) => Part {
                    root: Some(root),
                    representation: "json".into(),
                    state: state.into(),
                    pairs: false,
                    original: None,
                },
                Err(_) => Part {
                    root: Some(Value::String(text.clone())),
                    representation: "text".into(),
                    state: if state == "full" && media.as_deref().is_some_and(is_json_media) {
                        "unreadable_structure"
                    } else {
                        state
                    }
                    .into(),
                    pairs: false,
                    original: None,
                },
            }
        }
    }
}
fn pairs(encoded: &str) -> Value {
    Value::Array(
        encoded
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                json!([decode(k, true), decode(v, true)])
            })
            .collect(),
    )
}
fn decode(encoded: &str, plus: bool) -> String {
    let bytes = encoded.as_bytes();
    let mut output = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus => output.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let byte = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok());
                if let Some(byte) = byte {
                    output.push(byte);
                    i += 2;
                } else {
                    output.push(bytes[i]);
                }
            }
            b => output.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}
