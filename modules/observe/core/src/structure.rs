//! What one exchange looks like, without its values. Equal structures share
//! a fingerprint.

use nexofolio_contracts::endpoint::FieldLocation;
use nexofolio_contracts::observation::{Body, Content, HttpExchange};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::shape::Shape;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Structure {
    /// `None` when no response was captured.
    pub status: Option<u16>,
    pub query: Part,
    pub request: Part,
    pub response: Part,
    #[serde(default)]
    pub request_headers: Part,
    #[serde(default)]
    pub response_headers: Part,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    #[serde(default)]
    pub state: Option<String>,
    pub media_type: Option<String>,
    pub shape: Shape,
    /// The shape is the whole content, so a missing field is really missing.
    pub complete: bool,
}

impl Structure {
    pub fn of(exchange: &HttpExchange) -> Self {
        let request = &exchange.request;
        let keys: Vec<String> = pairs(query_of(&request.url).unwrap_or_default()).collect();
        Self {
            status: exchange.response.as_ref().map(|response| response.status),
            query: Part {
                state: None,
                media_type: None,
                shape: Shape::of_keys(keys.iter().map(String::as_str)),
                complete: !request.url_truncated,
            },
            request: Part::of(&request.body),
            request_headers: Part::headers(request.headers.as_ref()),
            response_headers: Part::headers(
                exchange.response.as_ref().and_then(|r| r.headers.as_ref()),
            ),
            response: exchange
                .response
                .as_ref()
                .map_or_else(Part::default, |response| Part::of(&response.body)),
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        let bytes = serde_json::to_vec(self).expect("structures serialise");
        Sha256::digest(bytes).into()
    }

    /// The part holding fields of `location`; `None` when this exchange has no
    /// such part (another status, or path parameters).
    pub fn part(&self, location: FieldLocation) -> Option<&Part> {
        match location {
            FieldLocation::Path => None,
            FieldLocation::Query => Some(&self.query),
            FieldLocation::RequestBody => Some(&self.request),
            FieldLocation::RequestHeader => Some(&self.request_headers),
            FieldLocation::ResponseHeader { status } => {
                (self.status == Some(status)).then_some(&self.response_headers)
            }
            FieldLocation::ResponseBody { status } => {
                (self.status == Some(status)).then_some(&self.response)
            }
        }
    }

    /// Every part with the location its fields live in.
    pub fn parts(&self) -> Vec<(FieldLocation, &Part)> {
        let mut parts = vec![
            (FieldLocation::Query, &self.query),
            (FieldLocation::RequestHeader, &self.request_headers),
            (FieldLocation::RequestBody, &self.request),
        ];
        if let Some(status) = self.status {
            parts.push((FieldLocation::ResponseBody { status }, &self.response));
            parts.push((
                FieldLocation::ResponseHeader { status },
                &self.response_headers,
            ));
        }
        parts
    }
}

impl Part {
    fn headers(headers: Option<&nexofolio_contracts::observation::Headers>) -> Self {
        let Some(headers) = headers else {
            return Self::default();
        };
        let names: Vec<_> = headers
            .entries
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();
        Self {
            state: None,
            media_type: None,
            shape: Shape::of_keys(names.iter().map(String::as_str)),
            complete: headers.complete,
        }
    }
    fn of(body: &Body) -> Self {
        let (media_type, content) = match body {
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
            } => (media_type, Some(content)),
            Body::Unreadable { media_type, .. } => (media_type, None),
            Body::None => (&None, None),
        };
        let media_type = media_type.as_deref().map(essence);
        let shape = content.and_then(|content| shape_of(media_type.as_deref(), &text(content)));
        Self {
            state: Some(
                match body {
                    Body::Full { .. } => "full",
                    Body::Partial { .. } => "partial",
                    Body::Truncated { .. } => "truncated",
                    Body::Unreadable { .. } => "unreadable",
                    Body::None => "none",
                }
                .into(),
            ),
            complete: shape.is_some() && body.proves_absence(),
            shape: shape.unwrap_or_default(),
            media_type,
        }
    }
}

/// `application/json; charset=utf-8` → `application/json`.
pub fn essence(media_type: &str) -> String {
    let essence = media_type.split(';').next().unwrap_or_default();
    essence.trim().to_ascii_lowercase()
}

pub fn is_json_media(media_type: &str) -> bool {
    media_type == "application/json" || media_type.ends_with("+json") || media_type == "text/json"
}

fn text(content: &Content) -> std::borrow::Cow<'_, str> {
    match content {
        Content::Text(text) => text.into(),
        Content::Binary(bytes) => String::from_utf8_lossy(bytes),
    }
}

/// JSON, form keys, or nothing we can see into.
fn shape_of(media_type: Option<&str>, text: &str) -> Option<Shape> {
    match media_type {
        Some("application/x-www-form-urlencoded") => {
            let keys: Vec<String> = pairs(text).collect();
            Some(Shape::of_keys(keys.iter().map(String::as_str)))
        }
        Some("multipart/form-data") => {
            let names = if let Ok(Value::Array(fields)) = serde_json::from_str::<Value>(text) {
                fields
                    .iter()
                    .filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            } else {
                multipart_names(text)
            };
            Some(Shape::of_keys(names.iter().map(String::as_str)))
        }
        _ => {
            let json_media = media_type.is_none_or(is_json_media);
            let value: Value = serde_json::from_str(text).ok()?;
            (json_media || value.is_object() || value.is_array()).then(|| Shape::of(&value))
        }
    }
}

fn query_of(url: &str) -> Option<&str> {
    let before_fragment = url.split('#').next().unwrap_or_default();
    before_fragment.split_once('?').map(|(_, query)| query)
}

/// The path of an absolute URL, without query or fragment; `/` when empty.
pub fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    rest.find('/').map_or("/", |start| &rest[start..])
}

/// Decoded keys of `a=1&b=2`, deduplicated, in order.
fn pairs(encoded: &str) -> impl Iterator<Item = String> + '_ {
    let mut seen = Vec::new();
    encoded
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| decode(pair.split('=').next().unwrap_or_default()))
        .filter(move |key| {
            let fresh = !seen.contains(key);
            if fresh {
                seen.push(key.clone());
            }
            fresh
        })
}

fn multipart_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("content-disposition:") {
            continue;
        }
        let Some(start) = lower.find(" name=\"").map(|at| at + 7) else {
            continue;
        };
        if let Some(len) = line[start..].find('"') {
            let name = line[start..start + len].to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Form decoding: `+` is a space, `%XX` a byte; bad escapes stay as they are.
fn decode(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                Some(byte) => {
                    out.push(byte);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexofolio_contracts::endpoint::PathSegment;
    use nexofolio_contracts::observation::{HttpRequest, HttpResponse};

    use crate::shape::Presence;

    fn exchange(url: &str, request: Body, response: Option<(u16, Body)>) -> HttpExchange {
        HttpExchange {
            request: HttpRequest {
                method: "POST".into(),
                url: url.into(),
                url_truncated: false,
                headers: None,
                body: request,
            },
            response: response.map(|(status, body)| HttpResponse {
                status,
                headers: None,
                body,
            }),
        }
    }

    fn full(media_type: &str, text: &str) -> Body {
        Body::Full {
            media_type: Some(media_type.into()),
            content: Content::Text(text.into()),
        }
    }

    fn key(name: &str) -> Vec<PathSegment> {
        vec![PathSegment::Key(name.into())]
    }

    #[test]
    fn extracts_query_request_and_response() {
        let structure = Structure::of(&exchange(
            "https://api.example.com/order?page=1&size=2&page=3&na%6De=x#top",
            full("application/x-www-form-urlencoded", "a=1&b+c=2"),
            Some((200, full("application/json; charset=utf-8", r#"{"id":1}"#))),
        ));
        assert_eq!(
            structure.query.shape.fields.keys().collect::<Vec<_>>(),
            ["name", "page", "size"]
        );
        assert!(structure.query.complete);
        assert_eq!(
            structure.request.shape.fields.keys().collect::<Vec<_>>(),
            ["a", "b c"]
        );
        assert_eq!(
            structure.response.media_type.as_deref(),
            Some("application/json")
        );
        let response = structure
            .part(FieldLocation::ResponseBody { status: 200 })
            .unwrap();
        assert_eq!(response.shape.at(&key("id")).0, Presence::Present);
        assert!(
            structure
                .part(FieldLocation::ResponseBody { status: 404 })
                .is_none()
        );
    }

    #[test]
    fn only_full_structured_bodies_are_complete() {
        let truncated = Body::Truncated {
            media_type: Some("application/json".into()),
            content: Content::Text(r#"{"id":1}"#.into()),
            original_bytes: Some(10_000),
        };
        let structure = Structure::of(&exchange("https://api.example.com/x", truncated, None));
        assert!(!structure.request.complete, "truncated proves nothing");
        assert_eq!(
            structure.request.shape.at(&key("other")).0,
            Presence::Absent
        );
        let html = Structure::of(&exchange(
            "https://api.example.com/x",
            full("text/html", "<html></html>"),
            None,
        ));
        assert!(!html.request.complete);
        assert_eq!(html.request.media_type.as_deref(), Some("text/html"));
        assert_eq!(html.request.shape, Shape::default());
        assert_eq!(html.status, None);
    }

    #[test]
    fn values_do_not_change_the_hash_but_shapes_do() {
        let of = |text: &str| {
            Structure::of(&exchange(
                "https://api.example.com/x?page=1",
                Body::None,
                Some((200, full("application/json", text))),
            ))
            .hash()
        };
        assert_eq!(of(r#"{"id":1,"a":"x"}"#), of(r#"{"a":"y","id":2}"#));
        assert_ne!(of(r#"{"id":1}"#), of(r#"{"id":"1"}"#));
    }

    #[test]
    fn multipart_keeps_field_names() {
        let body = "--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\r\nx\r\n--b\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\ny\r\n--b--";
        let structure = Structure::of(&exchange(
            "https://api.example.com/upload",
            full("multipart/form-data; boundary=b", body),
            None,
        ));
        assert_eq!(
            structure.request.shape.fields.keys().collect::<Vec<_>>(),
            ["file", "note"]
        );
    }

    #[test]
    fn paths_and_decoding() {
        assert_eq!(path_of("https://api.example.com"), "/");
        assert_eq!(path_of("https://api.example.com:8443/a/b?x=/c#d"), "/a/b");
        assert_eq!(decode("a%20b+c%zz%4"), "a b c%zz%4");
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;
    use nexofolio_contracts::observation::{Headers, HttpRequest};
    fn of(body: Body) -> Structure {
        Structure::of(&HttpExchange {
            request: HttpRequest {
                method: "POST".into(),
                url: "https://fixture.test/x".into(),
                url_truncated: false,
                headers: Some(Headers {
                    complete: false,
                    entries: vec![],
                }),
                body,
            },
            response: None,
        })
    }
    #[test]
    fn none_empty_unreadable_and_truncated_are_distinct() {
        let bodies = [
            Body::None,
            Body::Full {
                media_type: Some("application/json".into()),
                content: Content::Text("{}".into()),
            },
            Body::Unreadable {
                media_type: None,
                note: None,
            },
            Body::Truncated {
                media_type: Some("application/json".into()),
                content: Content::Text("{}".into()),
                original_bytes: None,
            },
        ];
        let hashes: std::collections::HashSet<_> =
            bodies.into_iter().map(|b| of(b).hash()).collect();
        assert_eq!(hashes.len(), 4);
    }
    #[test]
    fn multipart_summary_projects_known_parameters_without_proving_absence() {
        let shape=of(Body::Partial {media_type:Some("multipart/form-data".into()),content:Content::Text(r#"[{"name":"note","value":"fixture"},{"name":"upload","file":"fixture.txt","type":"text/plain","size":12}]"#.into()),note:Some("no file bytes".into())});
        assert!(!shape.request.complete);
        assert_eq!(
            shape
                .request
                .shape
                .fields
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["note", "upload"]
        );
    }
}
