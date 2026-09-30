//! Organize: all endpoints of a project → folder structure + placements.

use nexofolio_common::FolderId;
use nexofolio_contracts::knowledge::{Author, Command};
use serde::Deserialize;

use crate::{json_slice, prompt, Completions, CompletionError};

pub struct Organize<C> {
    completions: C,
    model: String,
}

#[derive(Debug, Deserialize)]
struct FolderSpec {
    name: String,
    summary: String,
    #[serde(rename = "match")]
    match_patterns: Vec<String>,
}

impl<C> Organize<C> {
    pub fn new(completions: C, model: String) -> Self {
        Self { completions, model }
    }
}

impl<C: Completions> Organize<C> {
    /// A whole project. Returns PutFolder and Place commands. Position is
    /// assigned in the order the model lists folders.
    pub async fn project(
        &self,
        summaries: &[nexofolio_contracts::endpoint::EndpointSummary],
    ) -> Result<Vec<Command>, CompletionError> {
        if summaries.is_empty() {
            return Ok(vec![]);
        }

        let p = prompt::for_organization(summaries);
        let reply = self.completions.complete(&p).await?;

        let Some(json_part) = json_slice(&reply) else {
            return Err(CompletionError(format!("no JSON in reply: {}", reply)));
        };

        let folders: Vec<FolderSpec> = serde_json::from_str(json_part)
            .map_err(|e| CompletionError(format!("JSON parse failed: {}", e)))?;

        let mut commands = Vec::new();

        let mut placed: Vec<bool> = vec![false; summaries.len()];

        for (idx, spec) in folders.into_iter().enumerate() {
            let folder_id = FolderId::new();

            commands.push(Command::PutFolder {
                id: folder_id,
                parent: None,
                name: spec.name,
                summary: Some(spec.summary),
                position: idx as i32,
            });

            // An endpoint sits in one folder, so the first folder that claims a
            // path keeps it: the model's order is its own priority order.
            for (i, endpoint_summary) in summaries.iter().enumerate() {
                if placed[i] {
                    continue;
                }
                let path = &endpoint_summary.path_template;
                if spec.match_patterns.iter().any(|pat| path.contains(pat)) {
                    placed[i] = true;
                    commands.push(Command::Place {
                        endpoint: endpoint_summary.id,
                        folder: folder_id,
                    });
                }
            }
        }

        Ok(commands)
    }

    pub fn author(&self) -> Author {
        Author::Curate {
            pass: "organize".into(),
            model: Some(self.model.clone()),
        }
    }
}
