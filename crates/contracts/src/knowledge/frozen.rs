//! Immutable, project-scoped content references. JSON text preserves original number tokens.
use super::{KnowledgeError, KnowledgeResult};
use nexofolio_common::{EndpointId, ProjectId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenRef {
    pub sha256: String,
    pub bytes: usize,
}
impl FrozenRef {
    pub fn of(content: &str) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
            bytes: content.len(),
        }
    }
    pub fn verify(&self, content: &str) -> KnowledgeResult<()> {
        if self != &Self::of(content) {
            return Err(frozen_invalid());
        }
        Ok(())
    }
    pub fn decode<T: DeserializeOwned>(&self, content: &str) -> KnowledgeResult<T> {
        self.verify(content)?;
        serde_json::from_str(content).map_err(|_| frozen_invalid())
    }
}
pub fn frozen_invalid() -> KnowledgeError {
    KnowledgeError::FrozenInputInvalid
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenEndpoint {
    pub facts: FrozenRef,
    pub inputs: Vec<FrozenFieldBlock>,
    pub values: FrozenRef,
    pub documents: BTreeMap<String, FrozenRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenFieldBlock {
    pub keys: Vec<String>,
    pub content: FrozenRef,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenManifest {
    pub version: u32,
    pub project: ProjectId,
    pub candidate: uuid::Uuid,
    /// Unique preparation attempt; abandoned attempts can never complete a different snapshot.
    pub snapshot: uuid::Uuid,
    pub endpoints: HashMap<EndpointId, FrozenEndpoint>,
    pub links: FrozenRef,
}
impl FrozenManifest {
    pub fn references(&self) -> impl Iterator<Item = &FrozenRef> {
        std::iter::once(&self.links).chain(self.endpoints.values().flat_map(|e| {
            [&e.facts, &e.values]
                .into_iter()
                .chain(e.inputs.iter().map(|b| &b.content))
                .chain(e.documents.values())
        }))
    }
}

/// Hash the complete v3 header, including ordered scope and recovery metadata, not only blocks.
/// Called only for the bounded manifest format, never a legacy prepared project.
pub fn frozen_input_hash(input: &serde_json::Value) -> String {
    let mut canonical = input.clone();
    if let Some(object) = canonical.as_object_mut() {
        object.remove("manifest_hash");
    }
    FrozenRef::of(&canonical.to_string()).sha256
}
