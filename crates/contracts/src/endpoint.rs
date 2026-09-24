//! Endpoint facts computed by observe, read by knowledge, curate, view and apps.
//!
//! Every project has exactly one document: one endpoint per identity, whatever
//! address or environment it was seen on. Identity is project + method + path
//! template. Endpoints of the project's own service addresses use a relative
//! template (`/order/{id}`); endpoints of external services carry their
//! address (`https://analytics.example.net/collect`) so the two never collide.
//! External endpoints are ordinary endpoints with an annotation, nothing less.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use serde::{Deserialize, Serialize};

use crate::scope::SiteScope;

/// One change to a project's endpoints, published on observe's change feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointEvent {
    pub project_id: ProjectId,
    pub endpoint: EndpointId,
    pub change: EndpointChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EndpointChange {
    Created {
        method: String,
        path_template: String,
    },
    /// A new structure was seen. `None` means a project-wide declaration changed.
    StructureChanged {
        environment_id: Option<EnvironmentId>,
    },
    /// `endpoint` is now an alias of `into`; references should move over.
    /// A renamed identity shows up as `Created` for the new one plus this.
    MergedInto { into: EndpointId },
}

/// Normalised origin of a service address, e.g. `https://api.example.com`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceAddress(String);

impl ServiceAddress {
    /// Accepts a bare origin (an optional trailing `/` is allowed), nothing more.
    pub fn parse(value: &str) -> Option<Self> {
        let origin = value.strip_suffix('/').unwrap_or(value);
        SiteScope::new(origin, "/")
            .ok()
            .map(|scope| Self(scope.origin().to_owned()))
    }

    /// The address an absolute request URL was sent to.
    pub fn of_url(url: &str) -> Option<Self> {
        SiteScope::origin_of(url).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ServiceAddress {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or(value)
    }
}

impl From<ServiceAddress> for String {
    fn from(address: ServiceAddress) -> Self {
        address.0
    }
}

impl fmt::Display for ServiceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Own,
    External,
}

/// Why observe decided a verdict on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoReason {
    /// Same registrable domain as the page the call was made from.
    SameSite,
    /// Seen in several unrelated projects.
    SharedAcrossProjects,
    /// Answers with something other than JSON.
    NotJson,
    /// Nothing decided it either way; kept as the project's own.
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    Auto(AutoReason),
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressStatus {
    pub address: ServiceAddress,
    pub verdict: Verdict,
    pub decision: Decision,
    pub calls: u64,
    /// `None` for addresses registered before any traffic arrived.
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentUsage {
    pub environment_id: EnvironmentId,
    pub calls: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSummary {
    pub id: EndpointId,
    pub project_id: ProjectId,
    pub method: String,
    pub path_template: String,
    /// Annotation only: served by an external service.
    pub external: bool,
    /// At least one declaration (e.g. Swagger) describes it.
    pub declared: bool,
    pub environments: Vec<EnvironmentUsage>,
}

/// Where an endpoint was called: address, environment and the base path
/// stripped before computing the template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressUse {
    pub environment_id: EnvironmentId,
    pub address: ServiceAddress,
    pub base_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "in", rename_all = "snake_case")]
pub enum FieldLocation {
    Path,
    Query,
    RequestBody,
    ResponseBody { status: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathSegment {
    Key(String),
    /// Every element of an array.
    Items,
}

/// Position of a field inside its location, e.g. `data.list[].sku`.
/// The empty path is the body itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(pub Vec<PathSegment>);

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Key(key) if index == 0 => f.write_str(key)?,
                PathSegment::Key(key) => write!(f, ".{key}")?,
                PathSegment::Items => f.write_str("[]")?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Null,
    Boolean,
    Number,
    String,
    Object,
    Array,
}

/// What the traffic of one environment says about a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "label", rename_all = "snake_case")]
pub enum FieldLabel {
    /// Too few calls to say anything.
    Observing,
    Always,
    /// Present in some calls and absent in others over the same period.
    Optional,
    /// Absent until `since`, present from then on: the endpoint changed.
    Added {
        since: DateTime<Utc>,
    },
    /// Present until `since`, absent from then on: the endpoint changed.
    Removed {
        since: DateTime<Utc>,
    },
    /// Several value types over the same period.
    Polymorphic,
    /// Never present in this environment, while calls prove it could be.
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentLabel {
    pub environment_id: EnvironmentId,
    pub label: FieldLabel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredField {
    pub required: bool,
    pub types: Vec<ValueType>,
}

/// A trusted declaration and the traffic disagree: the only case for a human.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Conflict {
    RequiredButAbsent { absent_calls: u64 },
    TypeMismatch { observed: Vec<ValueType> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldFacts {
    pub location: FieldLocation,
    pub path: FieldPath,
    pub types: Vec<ValueType>,
    pub labels: Vec<EnvironmentLabel>,
    /// Always present in one environment and never in another.
    pub differs_between_environments: bool,
    pub declared: Option<DeclaredField>,
    pub conflict: Option<Conflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointFacts {
    pub summary: EndpointSummary,
    /// Endpoints merged into this one; their IDs keep resolving here.
    pub aliases: Vec<EndpointId>,
    pub addresses: Vec<AddressUse>,
    pub fields: Vec<FieldFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// Retryable: storage is down.
    #[error("endpoint storage unavailable")]
    Unavailable,
}

/// Read model of endpoint facts. Implemented by observe.
#[async_trait]
pub trait EndpointReader: Send + Sync {
    /// Every current endpoint of a project; aliases are not listed.
    async fn list(&self, project: ProjectId) -> Result<Vec<EndpointSummary>, EndpointError>;

    /// One endpoint. An alias resolves to the endpoint it was merged into.
    async fn get(&self, id: EndpointId) -> Result<Option<EndpointFacts>, EndpointError>;
}

/// Which service addresses belong to a project. Implemented by observe.
/// Permission checks are the caller's job.
#[async_trait]
pub trait ServiceAddresses: Send + Sync {
    /// Addresses seen in the project's traffic or registered by hand.
    async fn list(&self, project: ProjectId) -> Result<Vec<AddressStatus>, EndpointError>;

    /// Sets a manual verdict, also for addresses not seen yet; `None` hands
    /// the decision back to observe. Existing endpoints follow the new
    /// verdict by merging, announced on the change feed.
    async fn decide(
        &self,
        project: ProjectId,
        address: ServiceAddress,
        verdict: Option<Verdict>,
    ) -> Result<(), EndpointError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_bare_normalised_origins() {
        let parsed = ServiceAddress::parse("HTTPS://API.Example.com:443/").unwrap();
        assert_eq!(parsed.as_str(), "https://api.example.com");
        assert!(ServiceAddress::parse("https://api.example.com/v1").is_none());
        assert!(ServiceAddress::parse("ftp://api.example.com").is_none());
        assert_eq!(
            ServiceAddress::of_url("https://api.example.com/order/1?x=1"),
            Some(parsed.clone())
        );
        let encoded = serde_json::to_string(&parsed).unwrap();
        assert_eq!(encoded, "\"https://api.example.com\"");
        assert!(serde_json::from_str::<ServiceAddress>("\"https://a.example.com/x\"").is_err());
    }

    #[test]
    fn field_paths_read_like_accessors() {
        let path = FieldPath(vec![
            PathSegment::Key("data".into()),
            PathSegment::Key("list".into()),
            PathSegment::Items,
            PathSegment::Key("sku".into()),
        ]);
        assert_eq!(path.to_string(), "data.list[].sku");
        assert_eq!(FieldPath(vec![PathSegment::Items]).to_string(), "[]");
        assert_eq!(FieldPath::default().to_string(), "");
    }
}
