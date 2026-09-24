//! Which project and environment collected data belongs to.
//!
//! Attribution is decided by the client and only validated here. A site scope
//! (origin + path prefix) records where the user was browsing; it annotates an
//! environment and never takes part in endpoint identity.

use std::fmt;

use async_trait::async_trait;
use nexofolio_common::{EnvironmentId, ProjectId};
use serde::{Deserialize, Serialize};

/// A page site scope such as `https://shop.example.com` + `/`.
///
/// The origin is normalised (lowercase scheme and host, default port dropped)
/// so the same site always compares equal. Prefixes match on whole path
/// segments: `/app` covers `/app` and `/app/x` but not `/apple`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawSiteScope", into = "RawSiteScope")]
pub struct SiteScope {
    origin: String,
    prefix: String,
}

#[derive(Serialize, Deserialize)]
struct RawSiteScope {
    origin: String,
    prefix: String,
}

impl TryFrom<RawSiteScope> for SiteScope {
    type Error = ScopeError;
    fn try_from(raw: RawSiteScope) -> Result<Self, ScopeError> {
        Self::new(&raw.origin, &raw.prefix)
    }
}

impl From<SiteScope> for RawSiteScope {
    fn from(scope: SiteScope) -> Self {
        Self {
            origin: scope.origin,
            prefix: scope.prefix,
        }
    }
}

impl SiteScope {
    pub fn new(origin: &str, prefix: &str) -> Result<Self, ScopeError> {
        let invalid = || ScopeError::InvalidSite(format!("{origin} {prefix}"));
        let (scheme, authority) = split_origin(origin).ok_or_else(invalid)?;
        if authority.contains(['/', '?', '#']) {
            return Err(invalid());
        }
        if !prefix.starts_with('/')
            || prefix.contains(['?', '#'])
            || prefix.contains(char::is_whitespace)
        {
            return Err(invalid());
        }
        Ok(Self {
            origin: normalise_origin(&scheme, authority),
            prefix: prefix.to_owned(),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The normalised origin of an absolute page URL, as scopes store it.
    pub fn origin_of(page_url: &str) -> Option<String> {
        split_page(page_url).map(|(origin, _)| origin)
    }

    /// Whether an absolute page URL falls inside this scope.
    pub fn contains(&self, page_url: &str) -> bool {
        let Some((origin, path)) = split_page(page_url) else {
            return false;
        };
        if origin != self.origin {
            return false;
        }
        match path.strip_prefix(self.prefix.as_str()) {
            None => false,
            Some(rest) => self.prefix.ends_with('/') || rest.is_empty() || rest.starts_with('/'),
        }
    }

    /// Longer prefixes win when several scopes contain the same URL.
    pub fn specificity(&self) -> usize {
        self.prefix.len()
    }
}

impl fmt::Display for SiteScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.origin, self.prefix)
    }
}

fn split_origin(value: &str) -> Option<(String, &str)> {
    let (scheme, rest) = value.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return None;
    }
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() || host.contains(char::is_whitespace) || host.contains('@') {
        return None;
    }
    Some((scheme, rest))
}

/// A page URL's normalised origin and its path, without query or fragment.
fn split_page(page_url: &str) -> Option<(String, &str)> {
    let (scheme, rest) = split_origin(page_url)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let tail = &rest[end..];
    let path = &tail[..tail.find(['?', '#']).unwrap_or(tail.len())];
    let path = if path.is_empty() { "/" } else { path };
    Some((normalise_origin(&scheme, &rest[..end]), path))
}

fn normalise_origin(scheme: &str, authority: &str) -> String {
    let authority = authority.to_ascii_lowercase();
    let default_port = if scheme == "https" { ":443" } else { ":80" };
    let authority = authority.strip_suffix(default_port).unwrap_or(&authority);
    format!("{scheme}://{authority}")
}

/// How a batch names its environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentSelector {
    /// Must already belong to the project.
    Id(EnvironmentId),
    /// Looked up by exact name, created when missing.
    Name(String),
}

/// The client's attribution claim for one batch (`target` in the collect contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectTarget {
    pub project_id: ProjectId,
    /// `None` only for declaration-only batches: the declarations then apply
    /// to every environment of the project.
    pub environment: Option<EnvironmentSelector>,
    pub site: Option<SiteScope>,
    pub source_url: Option<String>,
}

/// A validated target. The environment, when present, exists and belongs to the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTarget {
    pub project_id: ProjectId,
    pub environment_id: Option<EnvironmentId>,
    pub site: Option<SiteScope>,
    pub source_url: Option<String>,
}

/// One site scope maps to exactly one project + environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteBinding {
    pub site: SiteScope,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    #[error("unknown project")]
    UnknownProject,
    #[error("environment does not belong to the project")]
    UnknownEnvironment,
    #[error("invalid environment name: {0}")]
    InvalidEnvironmentName(String),
    #[error("invalid site scope: {0}")]
    InvalidSite(String),
    /// Retryable: storage or an upstream is down.
    #[error("scope storage unavailable")]
    Unavailable,
}

/// Validates collect targets. Implemented by access.
#[async_trait]
pub trait TargetResolver: Send + Sync {
    /// Checks the project and environment, creates a named environment when
    /// missing, and records `site` as an annotation of the environment.
    /// Calling it again with the same target must be harmless.
    async fn resolve(&self, target: &CollectTarget) -> Result<ResolvedTarget, ScopeError>;
}

/// The server-side site registry shared by every client. Implemented by access.
#[async_trait]
pub trait SiteRegistry: Send + Sync {
    /// The binding whose scope contains `page_url` with the longest prefix.
    async fn lookup(&self, page_url: &str) -> Result<Option<SiteBinding>, ScopeError>;

    /// Creates or replaces the binding for `binding.site`. The environment must
    /// belong to the project.
    async fn bind(&self, binding: SiteBinding) -> Result<(), ScopeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_normalised() {
        let a = SiteScope::new("HTTPS://Shop.Example.com:443", "/").unwrap();
        let b = SiteScope::new("https://shop.example.com", "/").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "https://shop.example.com/");
        assert_ne!(
            SiteScope::new("http://a.com:8080", "/").unwrap().origin(),
            "http://a.com"
        );
    }

    #[test]
    fn invalid_scopes_are_rejected() {
        assert!(SiteScope::new("https://a.com/login", "/").is_err());
        assert!(SiteScope::new("ftp://a.com", "/").is_err());
        assert!(SiteScope::new("https://", "/").is_err());
        assert!(SiteScope::new("https://a.com", "app").is_err());
        assert!(SiteScope::new("https://a.com", "/app?x=1").is_err());
    }

    #[test]
    fn prefixes_match_whole_segments() {
        let app = SiteScope::new("https://a.com", "/app").unwrap();
        assert!(app.contains("https://a.com/app"));
        assert!(app.contains("https://A.com:443/app/orders?page=1#top"));
        assert!(!app.contains("https://a.com/apple"));
        assert!(!app.contains("https://b.com/app"));
        assert!(!app.contains("http://a.com/app"));

        let root = SiteScope::new("https://a.com", "/").unwrap();
        assert!(root.contains("https://a.com"));
        assert!(root.contains("https://a.com/#/order/list"));
        assert!(root.specificity() < app.specificity());
    }

    #[test]
    fn page_origins_match_stored_origins() {
        assert_eq!(
            SiteScope::origin_of("HTTPS://A.com:443/app?x=1").as_deref(),
            Some("https://a.com")
        );
        assert_eq!(SiteScope::origin_of("a.com/app"), None);
    }

    #[test]
    fn serde_goes_through_validation() {
        let scope: SiteScope =
            serde_json::from_str(r#"{"origin":"https://A.com","prefix":"/"}"#).unwrap();
        assert_eq!(scope.origin(), "https://a.com");
        assert!(serde_json::from_str::<SiteScope>(r#"{"origin":"a.com","prefix":"/"}"#).is_err());
    }
}
