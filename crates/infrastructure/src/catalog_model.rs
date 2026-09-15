use async_trait::async_trait;
use nexofolio_contracts::*;
use nexofolio_rebuild::DirectoryGenerator;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub struct ChatCatalogGenerator {
    transport: crate::chat_transport::ChatTransport,
    model: String,
}
impl ChatCatalogGenerator {
    pub fn new(base_url: &str, key: Secret, model: String) -> Result<Self> {
        if model.trim().is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            transport: crate::chat_transport::ChatTransport::new(
                base_url,
                key,
                Duration::from_secs(120),
            )?,
            model,
        })
    }
}
fn system_prompt() -> String {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../contracts/catalog-preview/candidate.schema.json"
    ))
    .expect("bundled schema");
    let prompt = include_str!("../../../contracts/catalog-preview/prompt.txt");
    format!("{prompt}\nOutput JSON Schema:\n{schema}")
}
fn unavailable() -> Error {
    Error::Unavailable {
        component: "catalog_model",
    }
}
fn invalid() -> Error {
    Error::InvalidInput {
        message: "catalog model returned an invalid or incomplete candidate".into(),
    }
}
#[async_trait]
impl DirectoryGenerator for ChatCatalogGenerator {
    fn info(&self) -> Result<GeneratorInfo> {
        Ok(GeneratorInfo {
            adapter: "chat-completions-json".into(),
            model: self.model.clone(),
            prompt_version: "directory-generation-3".into(),
            prompt_sha256: Sha256::digest(system_prompt().as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        })
    }
    async fn generate(&self, snapshot: &CatalogSnapshot) -> Result<DirectoryCandidate> {
        let input = serde_json::to_string(snapshot).expect("serializes");
        if input.len() > MAX_PREVIEW_BYTES || snapshot.interfaces.len() > MAX_PREVIEW_INTERFACES {
            return Err(invalid());
        }
        let request = json!({"model":self.model,"stream":false,"response_format":{"type":"json_object"},"max_tokens":16384,"messages":[{"role":"system","content":system_prompt()},{"role":"user","content":input}]});
        let result = self
            .transport
            .complete(&request, MAX_CANDIDATE_BYTES * 2)
            .await
            .map_err(|_| unavailable())?;
        if result["choices"][0]["finish_reason"] != "stop" {
            return Err(invalid());
        }
        let content = result["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(invalid)?;
        if content.len() > MAX_CANDIDATE_BYTES {
            return Err(invalid());
        }
        let candidate: DirectoryCandidate = serde_json::from_str(content).map_err(|_| invalid())?;
        if candidate.nodes.len() > 500 || candidate.assignments.len() > MAX_PREVIEW_INTERFACES * 2 {
            return Err(invalid());
        }
        Ok(candidate)
    }
}
#[async_trait]
impl DirectoryGenerator for crate::Unconfigured {
    fn info(&self) -> Result<GeneratorInfo> {
        Err(Error::NotConfigured {
            capability: "catalog_model",
        })
    }
    async fn generate(&self, _: &CatalogSnapshot) -> Result<DirectoryCandidate> {
        Err(Error::NotConfigured {
            capability: "catalog_model",
        })
    }
}
