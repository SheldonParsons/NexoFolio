//! Model call/decoding only; directory identity, validation and diffing are pure planning.
use super::{directory_prompt, reply::json_slice};
use crate::{
    CompletionError, Completions,
    planning::catalogue::{Proposal, plan},
};
use nexofolio_contracts::{
    endpoint::EndpointSummary,
    knowledge::{Author, Command, EndpointNote, Folder, Placement},
};

pub struct Organize<C> {
    completions: C,
    model: String,
}
impl<C> Organize<C> {
    pub fn new(completions: C, model: String) -> Self {
        Self { completions, model }
    }
}
impl<C: Completions> Organize<C> {
    pub async fn project(
        &self,
        summaries: &[EndpointSummary],
    ) -> Result<Vec<Command>, CompletionError> {
        self.project_with_snapshot(summaries, &[], &[], &[]).await
    }
    pub async fn project_with_knowledge(
        &self,
        summaries: &[EndpointSummary],
        notes: &[EndpointNote],
        existing: &[Folder],
    ) -> Result<Vec<Command>, CompletionError> {
        self.project_with_snapshot(summaries, notes, existing, &[])
            .await
    }
    pub async fn project_with_snapshot(
        &self,
        summaries: &[EndpointSummary],
        notes: &[EndpointNote],
        existing: &[Folder],
        placements: &[Placement],
    ) -> Result<Vec<Command>, CompletionError> {
        if summaries.is_empty() {
            return Ok(vec![]);
        }
        let reply = self
            .completions
            .complete(&directory_prompt::build(
                summaries, notes, existing, placements,
            ))
            .await?;
        let json = json_slice(&reply)
            .ok_or_else(|| CompletionError("目录回复缺少 JSON，未写入".into()))?;
        let proposal: Proposal = serde_json::from_str(json)
            .map_err(|_| CompletionError("目录回复不符合两层结构，未写入".into()))?;
        plan(proposal, summaries, existing, placements)
    }
    pub fn author(&self) -> Author {
        Author::Curate {
            pass: "organize".into(),
            model: Some(self.model.clone()),
        }
    }
}
