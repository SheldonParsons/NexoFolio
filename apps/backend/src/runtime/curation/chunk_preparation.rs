//! Two passes: retain the complete global signal index, spill heavy documents per endpoint.
use super::{
    Curation, CurationError,
    frozen_input::{self, EndpointInput},
    jobs::Job,
    recovery::FrozenInput,
};
use nexofolio_common::ProjectId;
use nexofolio_contracts::{
    endpoint::EndpointSummary, field_values::FieldValueArchive, knowledge::*,
};
use nexofolio_curate::ProjectEvidence;
use std::collections::BTreeMap;

pub(super) async fn prepare(
    s: &Curation,
    project: ProjectId,
    summaries: &[EndpointSummary],
    job: &Job,
) -> Result<FrozenInput, CurationError> {
    let candidates = s.candidates.as_ref().ok_or(CurationError::Unavailable)?;
    let id = job
        .snapshot()
        .candidate_id
        .ok_or(CurationError::Unavailable)?;
    let candidate = candidates.candidate(project, id).await?;
    let lifecycle = candidate.base.lifecycle.clone();
    let baseline_batches = candidate.batches.iter().map(|b| b.id).collect();
    drop(candidate);
    let mut manifest = FrozenManifest {
        version: 1,
        project,
        candidate: id,
        snapshot: uuid::Uuid::new_v4(),
        endpoints: Default::default(),
        links: FrozenRef::of(""),
    };
    let mut signals = Vec::new();
    let mut skeleton = Vec::new();
    for summary in summaries {
        if lifecycle.endpoint(summary.id).is_some() {
            continue;
        }
        let e = super::snapshot::read_endpoint(s, summary, &lifecycle, Some(job)).await?;
        signals.extend(e.signals);
        skeleton.push(e.facts.summary.clone());
        let facts = frozen_input::write(
            s,
            &manifest,
            job.run_id,
            &EndpointInput {
                facts: e.facts,
                authentication: e.authentication,
            },
        )
        .await?;
        let mut values = e.values;
        let mut documents = BTreeMap::new();
        for (key, document) in std::mem::take(&mut values.documents) {
            documents.insert(
                key,
                frozen_input::write(s, &manifest, job.run_id, &document).await?,
            );
        }
        let values = frozen_input::write(s, &manifest, job.run_id, &values).await?;
        manifest.endpoints.insert(
            summary.id,
            FrozenEndpoint {
                facts,
                values,
                documents,
                inputs: vec![],
            },
        );
    }
    job.prepare_phase(
        super::preparation_progress::PreparationPhase::Analyzing,
        "资料已分块保存，正在核对完整跨接口证据",
    );
    let state = s.clone();
    let owner = job.run_id;
    let manifest = tokio::task::spawn_blocking(move || {
        // One CPU worker owns the borrowed index; all IO still uses the knowledge port.
        let handle = tokio::runtime::Handle::current();
        let evidence = ProjectEvidence::from_summaries(skeleton.clone(), &signals);
        manifest.links = handle.block_on(frozen_input::write(
            &state,
            &manifest,
            owner,
            &evidence.links(),
        ))?;
        for f in &skeleton {
            let refs = &manifest.endpoints[&f.id];
            let endpoint: EndpointInput =
                handle.block_on(frozen_input::decode(&state, &manifest, &refs.facts))?;
            let mut archive: FieldValueArchive =
                handle.block_on(frozen_input::decode(&state, &manifest, &refs.values))?;
            for (key, reference) in &refs.documents {
                archive.documents.insert(
                    key.clone(),
                    handle.block_on(frozen_input::decode(&state, &manifest, reference))?,
                );
            }
            let mut inputs = evidence.fields(&endpoint.facts);
            for input in &mut inputs {
                if let Some(values) = archive
                    .fields
                    .get(&nexofolio_observe::value_field_key(f.id, &input.key))
                {
                    input.observations = Some(nexofolio_observe::value_summary(values));
                    input.value_examples = nexofolio_curate::model_value_examples(&archive, values);
                }
            }
            let mut blocks = Vec::new();
            // Reuse the established model field limit; a field and its evidence are never truncated.
            for fields in inputs.chunks(nexofolio_curate::FIELD_BATCH_LIMIT) {
                let content =
                    handle.block_on(frozen_input::write(&state, &manifest, owner, &fields))?;
                blocks.push(FrozenFieldBlock {
                    keys: fields.iter().map(|i| i.key.clone()).collect(),
                    content,
                });
            }
            manifest
                .endpoints
                .get_mut(&f.id)
                .expect("prepared endpoint")
                .inputs = blocks;
        }
        Ok::<_, CurationError>(manifest)
    })
    .await
    .map_err(|_| CurationError::Failed("资料分析被中断，已保存内容保留".into()))??;
    let mut input = FrozenInput {
        version: 3,
        lifecycle_version: lifecycle.version,
        summaries: summaries.to_vec(),
        prepared: Default::default(),
        manifest_hash: None,
        manifest: Some(manifest),
        baseline_batches,
        legacy: false,
    };
    input.manifest_hash = Some(frozen_input_hash(
        &serde_json::to_value(&input).expect("manifest"),
    ));
    Ok(input)
}
