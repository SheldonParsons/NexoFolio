use nexofolio_common::{EndpointId, FolderId, ProjectId};
use nexofolio_contracts::{endpoint::EndpointSummary, knowledge::*};
use nexofolio_curate::{CompletionError, Completions, Organize};
use serde_json::{Value, json};
pub struct Canned(pub Value);
#[async_trait::async_trait]
impl Completions for Canned {
    async fn complete(&self, _: &str) -> Result<String, CompletionError> {
        Ok(self.0.to_string())
    }
}
pub fn summaries(count: usize) -> Vec<EndpointSummary> {
    let project = ProjectId::new();
    (0..count)
        .map(|i| EndpointSummary {
            id: EndpointId::new(),
            project_id: project,
            method: if i == 1 { "POST" } else { "GET" }.into(),
            path_template: "/gateway/shared/path".into(),
            declared: false,
            external: false,
            environments: vec![],
        })
        .collect()
}
pub fn author() -> Author {
    Author::Curate {
        pass: "organize".into(),
        model: Some("fixed".into()),
    }
}
pub fn folder(id: FolderId, parent: Option<FolderId>, name: &str, position: i32) -> Folder {
    Folder {
        id,
        parent,
        name: name.into(),
        summary: Some(format!("{name}用途")),
        position,
        endpoints: 0,
        endpoints_deep: 0,
        author: Some(author()),
    }
}
pub fn placement(endpoint: EndpointId, folder: FolderId) -> Placement {
    Placement {
        endpoint,
        folder,
        author: author(),
        at: "2026-10-07T00:00:00Z".parse().unwrap(),
    }
}
pub fn basic() -> Value {
    json!({"folders":[{"name":"账户","summary":"账户用途","children":[
        {"name":"查询","summary":"查询用途","endpoints":["0"]},
        {"name":"维护","summary":"维护用途","endpoints":["1"]}]}],"unplaced":[]})
}
pub async fn commands(
    proposal: Value,
    ss: &[EndpointSummary],
    folders: &[Folder],
    placements: &[Placement],
) -> Result<Vec<Command>, CompletionError> {
    Organize::new(Canned(proposal), "fixed".into())
        .project_with_snapshot(ss, &[], folders, placements)
        .await
}
