//! Read observe ports once per field pass; no SQL or semantic interpretation.
use super::{Curation, CurationError, jobs::Job};
use nexofolio_contracts::endpoint::{EndpointFacts, EndpointSummary};
use nexofolio_curate::{SampleField, sample_fields};

/// Read a bounded project evidence snapshot once. Each endpoint considers
/// related interfaces too; a stable own structure does not imply stable words.
pub(super) async fn evidence_snapshot(
    s: &Curation,
    summaries: &[EndpointSummary],
    job: Option<&Job>,
) -> Result<
    (
        Vec<EndpointFacts>,
        Vec<SampleField>,
        nexofolio_contracts::field_values::FieldValueArchive,
        std::collections::HashMap<
            nexofolio_common::EndpointId,
            Vec<nexofolio_contracts::knowledge::AuthenticationUse>,
        >,
    ),
    CurationError,
> {
    let mut facts = Vec::new();
    let mut signals = Vec::new();
    let mut archive = nexofolio_contracts::field_values::FieldValueArchive {
        version: 1,
        ..Default::default()
    };
    let mut authentication = std::collections::HashMap::new();
    let lifecycle = match summaries.first() {
        Some(su) => s.reader.lifecycle(su.project_id).await?,
        None => Default::default(),
    };
    for summary in summaries {
        if lifecycle.endpoint(summary.id).is_some() {
            continue;
        }
        let e = read_endpoint(s, summary, &lifecycle, job).await?;
        archive.documents.extend(e.values.documents);
        archive.fields.extend(e.values.fields);
        authentication.insert(summary.id, e.authentication);
        signals.extend(e.signals);
        facts.push(e.facts);
    }
    Ok((facts, signals, archive, authentication))
}

pub(super) struct EndpointEvidence {
    pub facts: EndpointFacts,
    pub signals: Vec<SampleField>,
    pub values: nexofolio_contracts::field_values::FieldValueArchive,
    pub authentication: Vec<nexofolio_contracts::knowledge::AuthenticationUse>,
}
pub(super) async fn read_endpoint(
    s: &Curation,
    summary: &EndpointSummary,
    lifecycle: &nexofolio_contracts::knowledge::DocumentLifecycle,
    job: Option<&Job>,
) -> Result<EndpointEvidence, CurationError> {
    let mut signals = Vec::new();
    let mut authentication = Vec::new();
    let mut endpoint = s
        .endpoints
        .get(summary.id)
        .await
        .map_err(|_| CurationError::Unavailable)?
        .ok_or(CurationError::NotFound)?;
    if endpoint.summary.project_id != summary.project_id {
        return Err(CurationError::NotFound);
    }
    if endpoint.summary.id != summary.id {
        return Err(CurationError::Conflict(
            "接口身份在准备资料期间发生变化；已保存成果保留，请开始新一轮整理".into(),
        ));
    }
    nexofolio_curate::active_facts(&mut endpoint, lifecycle);
    let mut retained = Vec::new();
    for example in &endpoint.examples {
        if let Some(raw) = s
            .endpoints
            .example(summary.id, &example.id)
            .await
            .map_err(|_| CurationError::Unavailable)?
        {
            let raw = nexofolio_curate::active_observation(
                raw,
                lifecycle,
                summary.id,
                &summary.path_template,
            );

            let endpoint_for_sample = endpoint.clone();
            let example_id = example.id.clone();
            let (raw, auth, fields) = tokio::task::spawn_blocking(move || {
                let auth = nexofolio_curate::authentication(&example_id, &raw);
                let fields = sample_fields(&endpoint_for_sample, &example_id, &raw);
                (raw, auth, fields)
            })
            .await
            .map_err(|_| CurationError::Failed("采集资料分析被中断，请重试。".into()))?;
            retained.push((example.id.clone(), Some(raw)));
            authentication.extend(auth);
            signals.extend(fields);
            if let Some(job) = job {
                job.update(|p| {
                    if let Some(preparation) = &mut p.preparation {
                        preparation.samples_read += 1;
                    }
                });
            }
        } else {
            retained.push((example.id.clone(), None));
            if let Some(job) = job {
                job.update(|p| {
                    if let Some(preparation) = &mut p.preparation {
                        preparation.samples_unavailable += 1;
                    }
                });
            }
        }
    }
    let value_facts = endpoint.clone();
    let values = tokio::task::spawn_blocking(move || {
        nexofolio_observe::archive_values(&value_facts, &retained)
    })
    .await
    .map_err(|_| CurationError::Failed("值证据提取被中断".into()))?;
    if let Some(job) = job {
        job.update(|p| {
            if let Some(preparation) = &mut p.preparation {
                preparation.endpoints_read += 1;
                preparation.fields += values.fields.len();
                preparation.values += values
                    .fields
                    .values()
                    .map(|f| f.values.len())
                    .sum::<usize>();
            }
        });
    }
    Ok(EndpointEvidence {
        facts: endpoint,
        signals,
        values,
        authentication,
    })
}
