//! Dispatch endpoint-local workers and retry directory proposals from acknowledged writes.
use super::{Curation, CurationError, activity::Activity, jobs::Job, preparation::PreparedProject};
use nexofolio_common::{EndpointId, ProjectId};
use nexofolio_contracts::endpoint::EndpointSummary;
use nexofolio_curate::Pass;
use tracing::Instrument;
pub(super) async fn run(
    s: &Curation,
    project: ProjectId,
    summaries: &[EndpointSummary],
    job: &Job,
    target: Option<EndpointId>,
    prepared: &PreparedProject,
    pass: Pass,
) -> Result<(), CurationError> {
    for attempt in 1..=3 {
        let result = async {
            match pass {
                Pass::Describe => super::descriptions::run(s, project, job, prepared).await,
                Pass::Fields => super::fields::run(s, project, job, target, prepared).await,
                Pass::Organize if target.is_none() => {
                    s.organize_snapshot(project, summaries, job).await
                }
                Pass::Organize => Ok(()),
            }
        }
        .instrument(tracing::info_span!("curation_stage",stage=?pass,attempt))
        .await;
        // Description failures retry only their endpoint; do not multiply budgets
        // or cancel/restart the entire stage after an exhausted endpoint retry.
        if pass != Pass::Organize {
            return result;
        }
        match result {
            Err(CurationError::Failed(message))
                if attempt < 3 && super::field_worker::error_code(&message) != "model_rejected" =>
            {
                job.update(|p| p.retries += 1);
                job.record(Activity::new(
                    pass,
                    "retry",
                    format!(
                        "本阶段出现临时失败或回复缺口，等待后第 {} 次核对；已保存内容会复用",
                        attempt + 1
                    ),
                ));
                s.checkpoint(
                    project,
                    job,
                    nexofolio_contracts::knowledge::CandidateState::Running,
                )
                .await?;
                tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
            }
            other => return other,
        }
    }
    unreachable!("third attempt returns")
}
