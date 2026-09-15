//! Current directory views and optimistic, idempotent activation commands.
use crate::{
    CatalogVersion, DirectoryId, EnvironmentId, InterfaceId, InterfaceMergeGroup, JobId, ProjectId,
    RevisionId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OfficialCatalog {
    pub project_id: ProjectId,
    pub generation: i64,
    pub version_id: Option<CatalogVersion>,
    pub source_task_id: Option<JobId>,
    pub source_run_id: Option<Uuid>,
    pub unclassified_id: DirectoryId,
    pub total_interfaces: i64,
    pub nodes: Vec<OfficialDirectory>,
    pub merge_groups: Vec<InterfaceMergeGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OfficialDirectory {
    pub id: DirectoryId,
    pub parent: Option<DirectoryId>,
    pub name: String,
    pub description: String,
    pub system: bool,
    pub locked: bool,
    pub direct_interfaces: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OfficialInterface {
    pub interface_id: InterfaceId,
    pub method: String,
    pub path: String,
    pub directory_id: DirectoryId,
    pub environments: Vec<OfficialEnvironment>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OfficialEnvironment {
    pub environment_id: EnvironmentId,
    pub environment_name: String,
    pub revision_id: RevisionId,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OfficialInterfacePage {
    pub items: Vec<OfficialInterface>,
    pub page: u32,
    pub limit: u32,
    pub total: i64,
    pub generation: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PublishedCatalogVersion {
    pub version_id: CatalogVersion,
    pub source_task_id: Option<JobId>,
    pub source_run_id: Option<Uuid>,
    pub created_at: String,
    pub current: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogVersionPage {
    pub items: Vec<PublishedCatalogVersion>,
    pub page: u32,
    pub limit: u32,
    pub total: i64,
    pub generation: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishCatalog {
    pub task_id: JobId,
    pub expected_generation: i64,
    pub request_id: Uuid,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreCatalog {
    #[serde(deserialize_with = "required_version")]
    #[schemars(required, schema_with = "nullable_version_schema")]
    pub version_id: Option<CatalogVersion>,
    pub expected_generation: i64,
    pub request_id: Uuid,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CatalogActivation {
    pub request_id: Uuid,
    pub generation: i64,
    pub version_id: Option<CatalogVersion>,
    pub replayed: bool,
}

fn required_version<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<CatalogVersion>, D::Error> {
    Deserialize::deserialize(d)
}
fn nullable_version_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::schema_for!(Option<CatalogVersion>)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_requires_explicit_target_even_for_initial_state() {
        let value = serde_json::json!({"request_id":Uuid::new_v4(),"expected_generation":0});
        assert!(serde_json::from_value::<RestoreCatalog>(value.clone()).is_err());
        let mut value = value;
        value["version_id"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<RestoreCatalog>(value).is_ok());
        let schema = serde_json::to_value(schemars::schema_for!(RestoreCatalog)).unwrap();
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "version_id")
        );
        assert!(
            schema["properties"]["version_id"]["type"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "null")
        );
    }
}
