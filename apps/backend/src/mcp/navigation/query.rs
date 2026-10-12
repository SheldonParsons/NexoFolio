//! Internal DTO. Public tool schemas restrict accepted keys independently.
use super::*;
use nexofolio_common::{FolderId, LinkId};
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Query {
    pub project_id: ProjectId,
    #[serde(default)]
    pub include_retired: bool,
    #[serde(skip)]
    pub lifecycle: nexofolio_contracts::knowledge::DocumentLifecycle,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub endpoint_id: Option<EndpointId>,
    #[serde(default)]
    pub directory_id: Option<FolderId>,
    #[serde(default)]
    pub link_id: Option<LinkId>,
    #[serde(default)]
    pub archive_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub field_key: Option<String>,
    #[serde(default)]
    pub parent_key: Option<String>,
    #[serde(default)]
    pub location: Option<nexofolio_contracts::endpoint::FieldLocation>,
    #[serde(default)]
    pub value_id: Option<String>,
    #[serde(default)]
    pub example_id: Option<String>,
    #[serde(default)]
    pub source_record: Option<nexofolio_contracts::observation::ObservationKey>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub part: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub unclassified: bool,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub issue_kind: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub business_scope: Option<String>,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub search_in: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub field_offset: usize,
    #[serde(default)]
    pub link_offset: usize,
    #[serde(default)]
    pub content_offset: usize,
    #[serde(default = "limit")]
    pub limit: usize,
    #[serde(default = "max_chars")]
    pub max_chars: usize,
    #[serde(default)]
    pub known_inputs: Vec<InputRef>,
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default = "depth")]
    pub max_depth: usize,
    #[serde(default = "steps")]
    pub max_steps: usize,
}
fn limit() -> usize {
    20
}
fn max_chars() -> usize {
    16384
}
fn depth() -> usize {
    4
}
fn steps() -> usize {
    60
}
impl Query {
    pub(super) fn validate(&self) -> Result<(), ReadError> {
        fn bound(
            p: &'static str,
            n: usize,
            min: usize,
            max: usize,
            suggested: usize,
            unit: &'static str,
        ) -> Result<(), ReadError> {
            if n < min || n > max {
                return Err(ReadError::bounds(p, n, min, max, suggested, unit));
            }
            Ok(())
        }
        bound("limit", self.limit, 1, 50, 20, "items")?;
        bound("max_chars", self.max_chars, 256, 16384, 16384, "characters")?;
        bound(
            "offset",
            self.offset,
            0,
            usize::MAX - self.limit,
            0,
            "items",
        )?;
        bound("field_offset", self.field_offset, 0, 100000, 0, "items")?;
        bound("link_offset", self.link_offset, 0, 100000, 0, "items")?;
        if self.query.chars().count() > 200 {
            return Err(ReadError::length(
                "query",
                self.query.chars().count(),
                0,
                200,
            ));
        }
        bound(
            "known_inputs",
            self.known_inputs.len(),
            0,
            100,
            100,
            "items",
        )?;
        bound("max_depth", self.max_depth, 1, 8, 4, "levels")?;
        bound("max_steps", self.max_steps, 1, 100, 60, "steps")?;
        for input in &self.known_inputs {
            if input.path.chars().count() > 500 {
                return Err(ReadError::length(
                    "known_inputs.path",
                    input.path.chars().count(),
                    0,
                    500,
                ));
            }
            if !matches!(
                input.location,
                nexofolio_contracts::endpoint::FieldLocation::Path
                    | nexofolio_contracts::endpoint::FieldLocation::Query
                    | nexofolio_contracts::endpoint::FieldLocation::RequestHeader
                    | nexofolio_contracts::endpoint::FieldLocation::RequestBody
            ) {
                return Err("INVALID_BOUNDS".into());
            }
        }
        for (p, value, max) in [
            ("method", &self.method, 10),
            ("business_scope", &self.business_scope, 200),
            ("field_key", &self.field_key, 4096),
            ("parent_key", &self.parent_key, 4096),
            ("example_id", &self.example_id, 100),
            ("value_id", &self.value_id, 64),
        ] {
            if let Some(value) = value
                && value.chars().count() > max
            {
                return Err(ReadError::length(p, value.chars().count(), 0, max));
            }
        }
        if let Some(revision) = &self.revision
            && revision.chars().count() != 64
        {
            return Err(ReadError::length(
                "revision",
                revision.chars().count(),
                64,
                64,
            ));
        }
        Ok(())
    }
    pub(super) fn arguments(&self) -> Value {
        serde_json::to_value(self).expect("read query serializes")
    }
}
