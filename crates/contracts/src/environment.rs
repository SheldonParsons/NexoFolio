use crate::{EnvironmentId, Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EnvironmentRef {
    ById(EnvironmentById),
    ByName(EnvironmentByName),
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentById {
    pub id: EnvironmentId,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentByName {
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: String,
}
/// Exact case-sensitive names; no silent Unicode/case/whitespace normalization.
pub fn validate_environment_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 64
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput {
            message:
                "environment name must be 1..64 characters, no controls or surrounding whitespace"
                    .into(),
        });
    }
    Ok(())
}
