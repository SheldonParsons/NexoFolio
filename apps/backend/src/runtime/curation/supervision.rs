//! Poll work, lease renewal, cancellation and inactivity together, without a total run deadline.
use super::{Curation, CurationError, jobs::Job};
use nexofolio_common::ProjectId;
use std::{future::Future, time::Duration};

pub(super) async fn heartbeat(
    s: &Curation,
    project: ProjectId,
    job: &Job,
) -> Result<(), CurationError> {
    let mut interval = tokio::time::interval(Duration::from_secs(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let (Some(candidates), Some(id)) = (&s.candidates, job.snapshot().candidate_id) {
            tokio::time::timeout(
                Duration::from_secs(30),
                candidates.heartbeat_candidate(project, id, job.run_id),
            )
            .await
            .map_err(|_| {
                CurationError::Failed(
                    "运行状态确认超过 30 秒未完成，本轮已停止，已保存内容保留".into(),
                )
            })??;
        }
        job.heartbeat();
    }
}

pub(super) async fn supervise(
    job: &Job,
    run: impl Future<Output = Result<(), CurationError>>,
    heartbeat: impl Future<Output = Result<(), CurationError>>,
    idle_budget: Duration,
) -> Result<(), CurationError> {
    // Never await heartbeat inside a selected branch: run may hold the very
    // project transaction the heartbeat needs, and must keep being polled to commit.
    let stalled = async {
        let mut ticker = tokio::time::interval(idle_budget.min(Duration::from_secs(5)));
        let mut last_update = job.snapshot().updated_at;
        let mut changed = tokio::time::Instant::now();
        loop {
            ticker.tick().await;
            let updated = job.snapshot().updated_at;
            if updated != last_update {
                last_update = updated;
                changed = tokio::time::Instant::now();
            }
            if changed.elapsed() >= idle_budget {
                break;
            }
        }
    };
    tokio::select! {
        biased;
        _ = job.cancelled() => Err(CurationError::Failed("本轮已停止，已保存批次保留".into())),
        result = run => result,
        result = heartbeat => result,
        _ = stalled => Err(CurationError::Failed(format!("超过 {} 分钟没有处理进展，本轮已停止，已保存内容保留", idle_budget.as_secs()/60))),
    }
}
