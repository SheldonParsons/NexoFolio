//! Describe: one endpoint's method, path and fields → name + purpose.

use nexofolio_contracts::{
    endpoint::EndpointFacts,
    knowledge::{Author, Command},
};
use serde::Deserialize;

use crate::{json_slice, prompt, Completions, CompletionError};

pub struct Describe<C> {
    completions: C,
    model: String,
}

impl<C> Describe<C> {
    pub fn new(completions: C, model: String) -> Self {
        Self { completions, model }
    }
}

impl<C: Completions> Describe<C> {
    /// One endpoint. Returns one PutEndpointNote command if the model produced
    /// a name or a purpose; returns nothing if both are missing, so no empty
    /// note is written.
    pub async fn endpoint(&self, facts: &EndpointFacts) -> Result<Option<Command>, CompletionError> {
        let p = prompt::for_description(facts);
        let reply = self.completions.complete(&p).await?;
        let Some(json_part) = json_slice(&reply) else {
            return Err(CompletionError("no JSON in reply".into()));
        };

        let parsed: Parsed =
            serde_json::from_str(json_part).map_err(|e| CompletionError(e.to_string()))?;

        let name = if parsed.name.trim().is_empty() {
            None
        } else {
            Some(parsed.name.trim().to_string())
        };
        let purpose = if parsed.purpose.trim().is_empty() {
            None
        } else {
            Some(parsed.purpose.trim().to_string())
        };

        if name.is_none() && purpose.is_none() {
            return Ok(None);
        }

        Ok(Some(Command::PutEndpointNote {
            endpoint: facts.summary.id,
            name,
            purpose,
        }))
    }

    /// Many endpoints. Stops early if the model fails; a partial run writes
    /// partial results rather than forcing a full re-run.
    pub async fn many(
        &self,
        facts: &[EndpointFacts],
    ) -> Result<Vec<Command>, CompletionError> {
        let mut out = Vec::new();
        for f in facts {
            if let Some(cmd) = self.endpoint(f).await? {
                out.push(cmd);
            }
        }
        Ok(out)
    }

    pub fn author(&self) -> Author {
        Author::Curate {
            pass: "describe".into(),
            model: Some(self.model.clone()),
        }
    }
}

#[derive(Deserialize)]
struct Parsed {
    name: String,
    purpose: String,
}
