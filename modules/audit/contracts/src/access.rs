use chrono::{DateTime, Utc};
use nexofolio_common::{Error, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessQuery {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub kind: String,
    pub view: String,
    pub project: Option<String>,
    pub page_group: Option<String>,
    pub route: Option<String>,
    pub resource: Option<String>,
    pub action: Option<String>,
    pub method: Option<String>,
    pub actor: Option<String>,
    pub background: bool,
    pub page: u32,
    pub limit: u32,
}
impl Default for AccessQuery {
    fn default() -> Self {
        Self {
            since: Utc::now() - chrono::TimeDelta::days(7),
            until: Utc::now(),
            kind: "page".into(),
            view: "ranking".into(),
            project: None,
            page_group: None,
            route: None,
            resource: None,
            action: None,
            method: None,
            actor: None,
            background: false,
            page: 1,
            limit: 20,
        }
    }
}
impl AccessQuery {
    pub fn validate(&self) -> Result<()> {
        if self.since > self.until
            || self.until - self.since > chrono::TimeDelta::days(366)
            || !matches!(self.kind.as_str(), "page" | "api" | "mcp")
            || !matches!(self.view.as_str(), "ranking" | "visits" | "legacy")
            || self.page_group.as_ref().is_some_and(|group| {
                self.kind != "page"
                    || !matches!(group.as_str(), "home" | "projects" | "docs" | "other")
            })
            || self.page == 0
            || self.page > 100000
            || !(1..=100).contains(&self.limit)
            || [&self.project, &self.resource]
                .iter()
                .any(|v| v.as_ref().is_some_and(|v| v.len() > 36))
            || [&self.action, &self.method, &self.actor]
                .iter()
                .any(|v| v.as_ref().is_some_and(|v| v.len() > 64))
            || self.route.as_ref().is_some_and(|v| v.len() > 200)
        {
            return Err(Error::invalid("invalid access query"));
        }
        Ok(())
    }
}
