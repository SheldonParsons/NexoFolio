//! The collect v1 wire format (`contracts/collect/v1/batch.schema.json`) and
//! its translation into canonical observations.
//!
//! The types mirror the schema one to one and deny unknown fields. What serde
//! cannot express (patterns, lengths, ranges) is checked right after parsing.
//! The fixture test in `tests/fixtures.rs` keeps the two from drifting apart.

use std::collections::BTreeMap;
use std::io;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Utc};
use nexofolio_common::{EnvironmentId, ProjectId};
use nexofolio_contracts::observation::{
    Body, Content, DeclaredBody, DeclaredParameter, DeclaredRequest, DeclaredResponse, Fact,
    Headers, HttpDeclaration, HttpExchange, HttpRequest, HttpResponse,
};
use nexofolio_contracts::scope::{CollectTarget, EnvironmentSelector, SiteScope};
use nexofolio_intake_contracts::RejectReason;
use serde::de::IgnoredAny;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use uuid::Uuid;

pub const MAX_RECORDS: usize = 50;
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

pub const EXCHANGE: &str = "http_exchange";
pub const DECLARATION: &str = "http_declaration";

/// The batch envelope, checked. Records are parsed one by one later so a bad
/// record only rejects itself.
pub(crate) struct Batch {
    pub batch_id: Uuid,
    pub platform: String,
    pub platform_version: Option<String>,
    pub target: CollectTarget,
    pub records: Vec<Value>,
}

/// One accepted record.
pub(crate) struct Record {
    pub id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub fact: Fact,
    /// Stored as sent; validated only.
    pub context: Option<Value>,
}

pub(crate) type Rejected = (RejectReason, String);

/// `platform`, checked before anything else so rate limiting can use it.
pub(crate) fn platform_of(batch: &Value) -> Result<&str, String> {
    match batch.get("platform").and_then(Value::as_str) {
        Some(platform) if valid_platform(platform) => Ok(platform),
        Some(_) => Err("platform must match ^[a-z0-9][a-z0-9._-]{0,63}$".into()),
        None => Err("platform is required".into()),
    }
}

pub(crate) fn parse_batch(batch: Value) -> Result<Batch, String> {
    if !batch.is_object() {
        return Err("batch must be an object".into());
    }
    let has_exchange = batch
        .get("records")
        .and_then(Value::as_array)
        .is_some_and(|records| {
            records
                .iter()
                .any(|r| r.get("kind") == Some(&EXCHANGE.into()))
        });
    let wire = WireBatch::deserialize(batch).map_err(|e| e.to_string())?;

    let batch_id = strict_uuid(&wire.batch_id).ok_or("batch_id must be a UUID")?;
    if !(1..=MAX_RECORDS).contains(&wire.records.len()) {
        return Err(format!("records must hold 1 to {MAX_RECORDS} entries"));
    }
    if let Some(version) = &wire.platform_version {
        check_len("platform_version", version, 1, 64)?;
    }
    let target = wire.target.check()?;
    if has_exchange && target.environment.is_none() {
        return Err(
            "target.environment is required when the batch has http_exchange records".into(),
        );
    }
    Ok(Batch {
        batch_id,
        platform: wire.platform,
        platform_version: wire.platform_version,
        target,
        records: wire.records,
    })
}

/// The record's `id` as sent, for the receipt.
pub(crate) fn raw_id(record: &Value) -> String {
    record
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn parse_record(record: &Value) -> Result<Record, Rejected> {
    let invalid = |message: String| (RejectReason::InvalidRecord, message);
    if serialized_len_exceeds(record, MAX_RECORD_BYTES) {
        return Err((
            RejectReason::RecordTooLarge,
            format!("record exceeds {MAX_RECORD_BYTES} bytes"),
        ));
    }
    let id = record
        .get("id")
        .and_then(Value::as_str)
        .and_then(strict_uuid)
        .ok_or_else(|| invalid("id must be a UUID".into()))?;
    let kind = match record.get("kind") {
        Some(Value::String(kind)) => kind.as_str(),
        _ => return Err(invalid("kind must be a string".into())),
    };
    if kind != EXCHANGE && kind != DECLARATION {
        return Err((
            RejectReason::UnsupportedKind,
            format!("unsupported kind {kind:?}"),
        ));
    }
    match record.get("version") {
        None => return Err(invalid("version is required".into())),
        Some(version) if version.as_f64() != Some(1.0) => {
            return Err((
                RejectReason::UnsupportedVersion,
                format!("{kind} supports version 1 only"),
            ));
        }
        Some(_) => {}
    }

    let parsed = if kind == EXCHANGE {
        let wire = ExchangeRecord::deserialize(record).map_err(|e| e.to_string());
        wire.and_then(|wire| {
            if let Some(context) = &wire.context {
                context.check()?;
            }
            Ok((wire.observed_at, Fact::Exchange(wire.payload.check()?)))
        })
    } else {
        let wire = DeclarationRecord::deserialize(record).map_err(|e| e.to_string());
        wire.and_then(|wire| Ok((wire.observed_at, Fact::Declaration(wire.payload.check()?))))
    };
    let (observed_at, fact) = parsed.map_err(invalid)?;
    let observed_at = timestamp(&observed_at)
        .ok_or_else(|| invalid("observed_at must be RFC 3339 with a zone".into()))?;
    Ok(Record {
        id,
        observed_at,
        fact,
        context: record.get("context").cloned(),
    })
}

// --- envelope ---------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBatch {
    batch_id: String,
    platform: String,
    #[serde(default, deserialize_with = "present")]
    platform_version: Option<String>,
    target: WireTarget,
    records: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTarget {
    project_id: String,
    #[serde(default, deserialize_with = "present")]
    environment: Option<WireEnvironment>,
    /// `null` and absent both mean "no site".
    #[serde(default)]
    site: Option<WireSite>,
    #[serde(default, deserialize_with = "present")]
    source_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvironment {
    #[serde(default, deserialize_with = "present")]
    id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSite {
    origin: String,
    prefix: String,
}

impl WireTarget {
    fn check(self) -> Result<CollectTarget, String> {
        let project_id =
            strict_id::<ProjectId>(&self.project_id).ok_or("target.project_id must be a UUID")?;
        let environment = match self.environment {
            None => None,
            Some(WireEnvironment {
                id: Some(id),
                name: None,
            }) => Some(EnvironmentSelector::Id(
                strict_id::<EnvironmentId>(&id).ok_or("target.environment.id must be a UUID")?,
            )),
            Some(WireEnvironment {
                id: None,
                name: Some(name),
            }) => {
                if !valid_environment_name(&name) {
                    return Err("target.environment.name must be 1 to 64 characters, without surrounding spaces or control characters".into());
                }
                Some(EnvironmentSelector::Name(name))
            }
            Some(_) => return Err("target.environment needs exactly one of id or name".into()),
        };
        let site = match self.site {
            None => None,
            Some(site) => {
                if !(within(&site.origin, 0, 2048) && valid_origin(&site.origin)) {
                    return Err("target.site.origin must be a bare http(s) origin".into());
                }
                if !(within(&site.prefix, 0, 2048) && valid_path(&site.prefix)) {
                    return Err(
                        "target.site.prefix must be a path without query or fragment".into(),
                    );
                }
                Some(SiteScope::new(&site.origin, &site.prefix).map_err(|e| e.to_string())?)
            }
        };
        if let Some(url) = &self.source_url {
            check_len("target.source_url", url, 1, 2048)?;
        }
        Ok(CollectTarget {
            project_id,
            environment,
            site,
            source_url: self.source_url,
        })
    }
}

// --- records ----------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeRecord {
    #[serde(rename = "id")]
    _id: IgnoredAny,
    #[serde(rename = "kind")]
    _kind: IgnoredAny,
    #[serde(rename = "version")]
    _version: IgnoredAny,
    observed_at: String,
    #[serde(default, deserialize_with = "present")]
    context: Option<WireContext>,
    payload: WireExchange,
}

/// Declarations carry no browser context.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationRecord {
    #[serde(rename = "id")]
    _id: IgnoredAny,
    #[serde(rename = "kind")]
    _kind: IgnoredAny,
    #[serde(rename = "version")]
    _version: IgnoredAny,
    observed_at: String,
    payload: WireDeclaration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContext {
    #[serde(default, deserialize_with = "present")]
    page_url: Option<String>,
    #[serde(default, deserialize_with = "present")]
    page_title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    client_instance_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    page_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    frame_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    view_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    interaction_id: Option<String>,
    #[serde(default, deserialize_with = "present", rename = "seq")]
    _seq: Option<u64>,
    #[serde(default, deserialize_with = "present", rename = "transport")]
    _transport: Option<Transport>,
    #[serde(default, deserialize_with = "present", rename = "frame")]
    _frame: Option<Frame>,
    #[serde(default, deserialize_with = "present")]
    started_at: Option<String>,
    #[serde(default, deserialize_with = "present")]
    completed_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Transport {
    Xhr,
    Fetch,
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Frame {
    Page,
    Iframe,
}

impl WireContext {
    fn check(&self) -> Result<(), String> {
        optional_len("context.page_url", &self.page_url, 1024 * 1024)?;
        optional_len("context.page_title", &self.page_title, 1024)?;
        for (field, value) in [
            ("client_instance_id", &self.client_instance_id),
            ("page_id", &self.page_id),
            ("frame_id", &self.frame_id),
            ("view_id", &self.view_id),
            ("interaction_id", &self.interaction_id),
        ] {
            if value.as_deref().is_some_and(|id| strict_uuid(id).is_none()) {
                return Err(format!("context.{field} must be a UUID"));
            }
        }
        for (field, value) in [
            ("started_at", &self.started_at),
            ("completed_at", &self.completed_at),
        ] {
            if value.as_deref().is_some_and(|at| timestamp(at).is_none()) {
                return Err(format!("context.{field} must be RFC 3339 with a zone"));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireExchange {
    request: WireRequest,
    #[serde(default, deserialize_with = "present")]
    response: Option<WireResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    method: String,
    url: String,
    #[serde(default, deserialize_with = "present")]
    url_truncated: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    headers: Option<WireHeaders>,
    body: WireBody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    status: u16,
    #[serde(default, deserialize_with = "present")]
    headers: Option<WireHeaders>,
    body: WireBody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireHeaders {
    state: HeaderState,
    entries: Vec<(String, String)>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum HeaderState {
    Complete,
    Partial,
    Truncated,
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum WireBody {
    Full {
        #[serde(default, deserialize_with = "present")]
        media_type: Option<String>,
        encoding: Encoding,
        content: String,
        /// Redundant for a full body; accepted and dropped.
        #[serde(default, deserialize_with = "present", rename = "bytes")]
        _bytes: Option<u64>,
    },
    Truncated {
        #[serde(default, deserialize_with = "present")]
        media_type: Option<String>,
        encoding: Encoding,
        content: String,
        #[serde(default, deserialize_with = "present")]
        bytes: Option<u64>,
    },
    Unreadable {
        #[serde(default, deserialize_with = "present")]
        media_type: Option<String>,
        #[serde(default, deserialize_with = "present")]
        note: Option<String>,
    },
    /// A struct variant, so unknown fields are refused like everywhere else.
    None {},
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Encoding {
    Utf8,
    Base64,
}

impl WireExchange {
    fn check(self) -> Result<HttpExchange, String> {
        let request = self.request;
        if !valid_method(&request.method) {
            return Err("request.method must match ^[A-Z][A-Z0-9_-]{0,15}$".into());
        }
        if !(within(&request.url, 0, 1024 * 1024) && absolute_url(&request.url)) {
            return Err("request.url must be an absolute http(s) URL".into());
        }
        let response = match self.response {
            None => None,
            Some(response) => {
                if !(100..=599).contains(&response.status) {
                    return Err("response.status must be 100 to 599".into());
                }
                Some(HttpResponse {
                    status: response.status,
                    headers: response.headers.map(WireHeaders::check).transpose()?,
                    body: response.body.check()?,
                })
            }
        };
        Ok(HttpExchange {
            request: HttpRequest {
                method: request.method,
                url: request.url,
                url_truncated: request.url_truncated.unwrap_or(false),
                headers: request.headers.map(WireHeaders::check).transpose()?,
                body: request.body.check()?,
            },
            response,
        })
    }
}

impl WireHeaders {
    fn check(self) -> Result<Headers, String> {
        if self.entries.len() > 256 {
            return Err("headers.entries holds at most 256 entries".into());
        }
        for (name, value) in &self.entries {
            check_len("header name", name, 1, 1024)?;
            check_len("header value", value, 0, 65536)?;
        }
        Ok(Headers {
            complete: self.state == HeaderState::Complete,
            entries: self.entries,
        })
    }
}

impl WireBody {
    fn check(self) -> Result<Body, String> {
        let media = |media_type: &Option<String>| optional_len("body.media_type", media_type, 255);
        Ok(match self {
            Self::Full {
                media_type,
                encoding,
                content,
                _bytes,
            } => {
                media(&media_type)?;
                Body::Full {
                    media_type,
                    content: decode(encoding, content)?,
                }
            }
            Self::Truncated {
                media_type,
                encoding,
                content,
                bytes,
            } => {
                media(&media_type)?;
                Body::Truncated {
                    media_type,
                    content: decode(encoding, content)?,
                    original_bytes: bytes,
                }
            }
            Self::Unreadable { media_type, note } => {
                media(&media_type)?;
                optional_len("body.note", &note, 1024)?;
                Body::Unreadable { media_type, note }
            }
            Self::None {} => Body::None,
        })
    }
}

fn decode(encoding: Encoding, content: String) -> Result<Content, String> {
    match encoding {
        Encoding::Utf8 => Ok(Content::Text(content)),
        Encoding::Base64 => BASE64
            .decode(content)
            .map(Content::Binary)
            .map_err(|_| "body.content is not valid base64".into()),
    }
}

// --- declarations -----------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDeclaration {
    method: String,
    path: String,
    #[serde(default, deserialize_with = "present")]
    operation_id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    summary: Option<String>,
    #[serde(default, deserialize_with = "present")]
    description: Option<String>,
    #[serde(default, deserialize_with = "present")]
    tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    deprecated: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    request: Option<WireDeclaredRequest>,
    #[serde(default, deserialize_with = "present")]
    responses: Option<BTreeMap<String, WireDeclaredResponse>>,
}

type WireParameters = BTreeMap<String, WireParameter>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDeclaredRequest {
    #[serde(default, deserialize_with = "present")]
    path_params: Option<WireParameters>,
    #[serde(default, deserialize_with = "present")]
    query: Option<WireParameters>,
    #[serde(default, deserialize_with = "present")]
    headers: Option<WireParameters>,
    #[serde(default, deserialize_with = "present")]
    body: Option<WireDeclaredBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireParameter {
    #[serde(default, deserialize_with = "present")]
    required: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    description: Option<String>,
    #[serde(default, deserialize_with = "present")]
    schema: Option<Map<String, Value>>,
    /// Any value, `null` included.
    #[serde(default)]
    example: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDeclaredBody {
    #[serde(default, deserialize_with = "present")]
    required: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    media_type: Option<String>,
    #[serde(default, deserialize_with = "present")]
    schema: Option<Map<String, Value>>,
    #[serde(default)]
    example: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDeclaredResponse {
    #[serde(default, deserialize_with = "present")]
    description: Option<String>,
    #[serde(default, deserialize_with = "present")]
    media_type: Option<String>,
    #[serde(default, deserialize_with = "present")]
    schema: Option<Map<String, Value>>,
    #[serde(default)]
    example: Option<Value>,
}

impl WireDeclaration {
    fn check(self) -> Result<HttpDeclaration, String> {
        if !valid_method(&self.method) {
            return Err("method must match ^[A-Z][A-Z0-9_-]{0,15}$".into());
        }
        if !(within(&self.path, 0, 2048) && valid_path(&self.path)) {
            return Err("path must be a path template without query or fragment".into());
        }
        optional_len("operation_id", &self.operation_id, 256)?;
        optional_len("summary", &self.summary, 1024)?;
        optional_len("description", &self.description, 65536)?;
        let tags = self.tags.unwrap_or_default();
        if tags.len() > 32 || tags.iter().any(|tag| !within(tag, 0, 256)) {
            return Err("tags holds at most 32 tags of at most 256 characters".into());
        }
        let request = match self.request {
            None => DeclaredRequest::default(),
            Some(request) => DeclaredRequest {
                path_params: parameters("request.path_params", request.path_params)?,
                query: parameters("request.query", request.query)?,
                headers: parameters("request.headers", request.headers)?,
                body: match request.body {
                    None => None,
                    Some(body) => {
                        optional_len("request.body.media_type", &body.media_type, 255)?;
                        Some(DeclaredBody {
                            required: body.required.unwrap_or(false),
                            media_type: body.media_type,
                            schema: body.schema.map(Value::Object),
                            example: body.example,
                        })
                    }
                },
            },
        };
        let mut responses = BTreeMap::new();
        for (code, response) in self.responses.unwrap_or_default() {
            if !valid_response_key(&code) {
                return Err(format!(
                    "responses key {code:?} must be a status, 1XX..5XX or default"
                ));
            }
            optional_len("response description", &response.description, 4096)?;
            optional_len("response media_type", &response.media_type, 255)?;
            responses.insert(
                code,
                DeclaredResponse {
                    description: response.description,
                    media_type: response.media_type,
                    schema: response.schema.map(Value::Object),
                    example: response.example,
                },
            );
        }
        Ok(HttpDeclaration {
            method: self.method,
            path_template: self.path,
            operation_id: self.operation_id,
            summary: self.summary,
            description: self.description,
            tags,
            deprecated: self.deprecated.unwrap_or(false),
            request,
            responses,
        })
    }
}

fn parameters(
    field: &str,
    wire: Option<WireParameters>,
) -> Result<BTreeMap<String, DeclaredParameter>, String> {
    let wire = wire.unwrap_or_default();
    if wire.len() > 256 {
        return Err(format!("{field} holds at most 256 parameters"));
    }
    wire.into_iter()
        .map(|(name, parameter)| {
            optional_len("parameter description", &parameter.description, 4096)?;
            Ok((
                name,
                DeclaredParameter {
                    required: parameter.required.unwrap_or(false),
                    description: parameter.description,
                    schema: parameter.schema.map(Value::Object),
                    example: parameter.example,
                },
            ))
        })
        .collect()
}

// --- rules the schema states as patterns and lengths ------------------------

/// An optional field that, when present, must not be `null`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Lengths count characters, as JSON Schema does.
fn within(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.chars().count())
}

fn check_len(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    match within(value, min, max) {
        true => Ok(()),
        false => Err(format!("{field} must be {min} to {max} characters")),
    }
}

fn optional_len(field: &str, value: &Option<String>, max: usize) -> Result<(), String> {
    value
        .as_deref()
        .map_or(Ok(()), |value| check_len(field, value, 0, max))
}

/// Hyphenated form only, as the schema's `Uuid` pattern.
fn strict_uuid(value: &str) -> Option<Uuid> {
    (value.len() == 36)
        .then(|| Uuid::try_parse(value).ok())
        .flatten()
}

fn strict_id<T: std::str::FromStr>(value: &str) -> Option<T> {
    strict_uuid(value).and_then(|_| value.parse().ok())
}

fn valid_platform(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value.len() <= 64
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn valid_method(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && value.len() <= 16
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// Same rule as access: exact, 1..=64 characters, no surrounding whitespace,
/// no control characters.
fn valid_environment_name(value: &str) -> bool {
    let control = |c: char| c <= '\u{1f}' || c == '\u{7f}';
    within(value, 1, 64)
        && !value.chars().any(control)
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
}

/// `^https?://` followed by a non-empty authority.
fn authority_after_scheme(value: &str) -> Option<&str> {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let end = rest
        .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
        .unwrap_or(rest.len());
    (end > 0).then_some(&rest[end..])
}

/// `^https?://[^/?#\s]+`
fn absolute_url(value: &str) -> bool {
    authority_after_scheme(value).is_some()
}

/// `^https?://[^/?#\s]+$`
fn valid_origin(value: &str) -> bool {
    authority_after_scheme(value).is_some_and(str::is_empty)
}

/// `^/[^?#\s]*$`
fn valid_path(value: &str) -> bool {
    value.starts_with('/') && !value.contains(|c: char| matches!(c, '?' | '#') || c.is_whitespace())
}

/// `^([1-5][0-9]{2}|[1-5]XX|default)$`
fn valid_response_key(value: &str) -> bool {
    let b = value.as_bytes();
    value == "default"
        || b.len() == 3
            && (b'1'..=b'5').contains(&b[0])
            && (b[1..].iter().all(u8::is_ascii_digit) || &b[1..] == b"XX")
}

/// RFC 3339 with seconds and an explicit zone, as the schema's `Timestamp`.
fn timestamp(value: &str) -> Option<DateTime<Utc>> {
    let b = value.as_bytes();
    let digits = |from: usize, to: usize| b[from..to].iter().all(u8::is_ascii_digit);
    let date_time = b.len() >= 20
        && digits(0, 4)
        && b[4] == b'-'
        && digits(5, 7)
        && b[7] == b'-'
        && digits(8, 10)
        && b[10] == b'T'
        && digits(11, 13)
        && b[13] == b':'
        && digits(14, 16)
        && b[16] == b':'
        && digits(17, 19);
    if !date_time {
        return None;
    }
    let mut zone = &b[19..];
    if let Some(fraction) = zone.strip_prefix(b".") {
        let len = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=9).contains(&len) {
            return None;
        }
        zone = &fraction[len..];
    }
    let zone_ok = zone == b"Z"
        || zone.len() == 6
            && matches!(zone[0], b'+' | b'-')
            && zone[1..3].iter().all(u8::is_ascii_digit)
            && zone[3] == b':'
            && zone[4..].iter().all(u8::is_ascii_digit);
    if !zone_ok {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Whether the compact JSON form of `value` is longer than `limit` bytes.
/// Stops serialising as soon as it is.
fn serialized_len_exceeds(value: &Value, limit: usize) -> bool {
    struct Counter {
        written: usize,
        limit: usize,
    }
    impl io::Write for Counter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written += buf.len();
            match self.written > self.limit {
                true => Err(io::Error::other("limit exceeded")),
                false => Ok(buf.len()),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Counter { written: 0, limit }, value).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_follow_the_schema() {
        assert!(valid_platform("nexofolio-fetcher"));
        assert!(valid_platform("swagger.sync_2"));
        assert!(!valid_platform("Fetcher"));
        assert!(!valid_platform("-x"));
        assert!(!valid_platform(&"a".repeat(65)));

        assert!(valid_method("GET") && valid_method("M-SEARCH"));
        assert!(!valid_method("get") && !valid_method("1GET"));

        assert!(absolute_url("https://api.example.com/x?y=1"));
        assert!(absolute_url("http://api.example.com"));
        assert!(!absolute_url("/api/x") && !absolute_url("https:///x"));
        assert!(valid_origin("https://shop.example.com:8443"));
        assert!(!valid_origin("https://shop.example.com/login"));

        assert!(valid_path("/order/{id}") && !valid_path("/order?x=1") && !valid_path("order"));
        assert!(
            valid_response_key("200") && valid_response_key("4XX") && valid_response_key("default")
        );
        assert!(
            !valid_response_key("600") && !valid_response_key("2xx") && !valid_response_key("20")
        );

        assert!(valid_environment_name("测试环境"));
        assert!(valid_environment_name("pre prod"));
        assert!(!valid_environment_name(" test") && !valid_environment_name("a\tb"));
        assert!(!valid_environment_name(""));

        assert!(strict_uuid("5b0f6a52-1c2d-4c8e-9f35-2a1d7c9e4b10").is_some());
        assert!(strict_uuid("5b0f6a521c2d4c8e9f352a1d7c9e4b10").is_none());
    }

    #[test]
    fn timestamps_need_a_zone() {
        assert!(timestamp("2026-09-24T08:00:00Z").is_some());
        assert!(timestamp("2026-09-24T08:00:00.123456789+08:00").is_some());
        assert_eq!(
            timestamp("2026-09-24T16:00:00+08:00"),
            timestamp("2026-09-24T08:00:00Z")
        );
        assert!(timestamp("2026-09-24T08:00:00").is_none());
        assert!(timestamp("2026-09-24 08:00:00Z").is_none());
        assert!(timestamp("2026-02-30T08:00:00Z").is_none());
        assert!(timestamp("2026-09-24T08:00:00.Z").is_none());
    }

    #[test]
    fn record_size_stops_early() {
        let value = Value::String("x".repeat(100));
        assert!(serialized_len_exceeds(&value, 50));
        assert!(!serialized_len_exceeds(&value, 102));
    }
}
