//! One dual-format reader. No cache: only active endpoint/batch readers retain heavy input.
use super::{Curation, CurationError, preparation::PreparedProject};
use nexofolio_common::EndpointId;
use nexofolio_contracts::{endpoint::EndpointFacts, knowledge::*};
use nexofolio_curate::{FieldInput, LinkProposal};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
pub(super) struct EndpointInput {
    pub facts: EndpointFacts,
    pub authentication: Vec<AuthenticationUse>,
}
pub(super) struct LoadedEndpoint {
    pub facts: EndpointFacts,
    pub inputs: Vec<FieldInput>,
    pub authentication: Vec<AuthenticationUse>,
}
#[derive(Clone)]
pub(super) struct ExecutionInput {
    pub endpoints: Vec<EndpointId>,
    pub links: Arc<Vec<LinkProposal>>,
    source: Source,
}
#[derive(Clone)]
enum Source {
    Legacy(Arc<PreparedProject>),
    Chunks(Arc<FrozenManifest>),
}
impl ExecutionInput {
    pub fn legacy(prepared: PreparedProject) -> Self {
        Self {
            endpoints: prepared.facts.iter().map(|f| f.summary.id).collect(),
            links: Arc::new(prepared.links.clone()),
            source: Source::Legacy(Arc::new(prepared)),
        }
    }
    pub async fn chunked(
        s: &Curation,
        manifest: FrozenManifest,
        endpoints: Vec<EndpointId>,
    ) -> Result<Self, CurationError> {
        if manifest.version != 1
            || endpoints.len() != manifest.endpoints.len()
            || endpoints
                .iter()
                .any(|e| !manifest.endpoints.contains_key(e))
        {
            return Err(frozen_invalid().into());
        }
        // Validate every block before ANY model request or write, also after a process restart.
        for reference in manifest.references() {
            read(s, &manifest, reference).await?;
        }
        let links = decode(s, &manifest, &manifest.links).await?;
        Ok(Self {
            endpoints,
            links: Arc::new(links),
            source: Source::Chunks(Arc::new(manifest)),
        })
    }
    pub async fn endpoint(
        &self,
        s: &Curation,
        endpoint: EndpointId,
    ) -> Result<LoadedEndpoint, CurationError> {
        self.load(s, endpoint, None).await
    }
    pub async fn batch(
        &self,
        s: &Curation,
        endpoint: EndpointId,
        keys: &[String],
    ) -> Result<LoadedEndpoint, CurationError> {
        self.load(s, endpoint, Some(keys)).await
    }
    async fn load(
        &self,
        s: &Curation,
        endpoint: EndpointId,
        keys: Option<&[String]>,
    ) -> Result<LoadedEndpoint, CurationError> {
        match &self.source {
            Source::Legacy(p) => Ok(LoadedEndpoint {
                facts: p
                    .facts
                    .iter()
                    .find(|f| f.summary.id == endpoint)
                    .ok_or_else(frozen_invalid)?
                    .clone(),
                inputs: p
                    .inputs
                    .get(&endpoint)
                    .ok_or_else(frozen_invalid)?
                    .iter()
                    .filter(|i| keys.is_none_or(|keys| keys.contains(&i.key)))
                    .cloned()
                    .collect(),
                authentication: p.authentication.get(&endpoint).cloned().unwrap_or_default(),
            }),
            Source::Chunks(m) => {
                let refs = m.endpoints.get(&endpoint).ok_or_else(frozen_invalid)?;
                let e: EndpointInput = decode(s, m, &refs.facts).await?;
                if e.facts.summary.id != endpoint || e.facts.summary.project_id != m.project {
                    return Err(frozen_invalid().into());
                }
                let mut inputs = Vec::new();
                for block in refs
                    .inputs
                    .iter()
                    .filter(|b| keys.is_none_or(|keys| b.keys.iter().any(|k| keys.contains(k))))
                {
                    let fields: Vec<FieldInput> = decode(s, m, &block.content).await?;
                    if fields.iter().map(|i| &i.key).ne(block.keys.iter()) {
                        return Err(frozen_invalid().into());
                    }
                    inputs.extend(
                        fields
                            .into_iter()
                            .filter(|i| keys.is_none_or(|keys| keys.contains(&i.key))),
                    );
                }
                Ok(LoadedEndpoint {
                    facts: e.facts,
                    authentication: e.authentication,
                    inputs,
                })
            }
        }
    }
}
pub(super) async fn read(
    s: &Curation,
    manifest: &FrozenManifest,
    reference: &FrozenRef,
) -> Result<String, CurationError> {
    Ok(s.candidates
        .as_ref()
        .ok_or(CurationError::Unavailable)?
        .frozen_chunk(
            manifest.project,
            manifest.candidate,
            manifest.snapshot,
            reference,
        )
        .await?)
}
pub(super) async fn decode<T: DeserializeOwned>(
    s: &Curation,
    manifest: &FrozenManifest,
    reference: &FrozenRef,
) -> Result<T, CurationError> {
    Ok(reference.decode(&read(s, manifest, reference).await?)?)
}
pub(super) async fn write<T: Serialize>(
    s: &Curation,
    manifest: &FrozenManifest,
    owner: uuid::Uuid,
    value: &T,
) -> Result<FrozenRef, CurationError> {
    let content = serde_json::to_string(value)
        .map_err(|_| CurationError::Failed("冻结资料编码失败".into()))?;
    Ok(s.candidates
        .as_ref()
        .ok_or(CurationError::Unavailable)?
        .put_frozen_chunk(
            manifest.project,
            manifest.candidate,
            owner,
            manifest.snapshot,
            &content,
        )
        .await?)
}
