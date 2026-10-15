//! Passive bounded observations. This module never acquires model permits or changes tasks.
#[cfg(any(target_os = "linux", test))]
mod cgroup;
mod resources;
pub(crate) mod storage;
mod snapshot;
#[cfg(test)]
mod tests;
use crate::http::collect_telemetry::{IntakeSnapshot, IntakeTelemetry};
use chrono::{DateTime, Utc};
use nexofolio_common::{PoolSnapshot, PoolTelemetry};
use nexofolio_curate_contracts::telemetry::{ModelSnapshot, ModelTelemetry};
use nexofolio_knowledge_contracts::operations::{OperationsReader, TaskCount};
use serde::Serialize;
use snapshot::Snapshot;
use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(crate) const SAMPLE_SECONDS: u64 = 5;
#[derive(Clone, Serialize)]
pub(crate) struct ResourcePoint {
    at: DateTime<Utc>,
    rss_bytes: Option<u64>,
    cpu_cores: Option<f32>,
}
#[derive(Clone, Serialize)]
pub(crate) struct IntakeRate {
    totals: IntakeSnapshot,
    window_seconds: Option<f64>,
    batches_per_second: Option<f64>,
    bytes_per_second: Option<f64>,
}
#[derive(Clone, Serialize)]
pub(crate) struct Summary {
    pub instance_id: uuid::Uuid,
    pub process_id: u32,
    pub started_at: DateTime<Utc>,
    pub sampled_at: Option<DateTime<Utc>>,
    pub sample_interval_seconds: u64,
    pub tasks: Snapshot<Vec<TaskCount>>,
    pub model: Snapshot<ModelSnapshot>,
    pub pools: Snapshot<Vec<PoolSnapshot>>,
    pub intake: Snapshot<IntakeRate>,
    pub resources: Option<resources::Resources>,
    pub history: VecDeque<ResourcePoint>,
}
#[derive(Clone)]
pub(crate) struct Monitor {
    cache: Arc<RwLock<Summary>>,
    pub storage: Option<storage::StorageMonitor>,
    tasks: Arc<dyn OperationsReader>,
    model: Arc<dyn ModelTelemetry>,
    pools: Vec<Arc<dyn PoolTelemetry>>,
    pub intake: Arc<IntakeTelemetry>,
}
impl Monitor {
    pub fn new(
        tasks: Arc<dyn OperationsReader>,
        model: Arc<dyn ModelTelemetry>,
        pools: Vec<Arc<dyn PoolTelemetry>>,
    ) -> Self {
        Self {
            cache: Arc::new(RwLock::new(Summary {
                instance_id: uuid::Uuid::new_v4(),
                process_id: std::process::id(),
                started_at: Utc::now(),
                sampled_at: None,
                sample_interval_seconds: SAMPLE_SECONDS,
                tasks: Snapshot::new(
                    "knowledge.curation_tasks grouped counts; background every 15s",
                    "database_global",
                    45,
                ),
                model: Snapshot::new(
                    "shared LlmClient HTTP, budget and last 256 completed timings",
                    "api_instance; counters since process start",
                    20,
                ),
                pools: Snapshot::new(
                    "sqlx pool size / idle (non-atomic); waiting and timeouts not instrumented",
                    "api_instance_by_module",
                    20,
                ),
                intake: Snapshot::new(
                    "collect HTTP completed attempts; readable body bytes; includes retries/refusals",
                    "api_instance; Fetcher local backlog unknown",
                    20,
                ),
                resources: None,
                history: VecDeque::new(),
            })),
            storage: None,
            tasks,
            model,
            pools,
            intake: Arc::new(IntakeTelemetry::default()),
        }
    }
    pub fn summary(&self) -> Option<Summary> {
        self.cache.read().ok().map(|s| s.clone())
    }
    pub async fn run(self, shutdown: CancellationToken) {
        let mut sampler = resources::Sampler::new();
        let mut interval = tokio::time::interval(Duration::from_secs(SAMPLE_SECONDS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut tick = 0u64;
        let mut previous_intake: Option<(DateTime<Utc>, IntakeSnapshot)> = None;
        loop {
            tokio::select! { _ = shutdown.cancelled() => break, _ = interval.tick() => {} }
            let model = self
                .model
                .model_snapshot()
                .ok_or("model_telemetry_unavailable");
            let pools = self.pools.iter().flat_map(|p| p.pool_snapshots()).collect();
            let sample = tokio::task::spawn_blocking(move || {
                let value = sampler.sample();
                (sampler, value)
            })
            .await;
            let Ok((returned, resources)) = sample else {
                break;
            };
            sampler = returned;
            let now = Utc::now();
            let intake = self
                .intake
                .snapshot()
                .map(|totals| {
                    let seconds = previous_intake
                        .as_ref()
                        .map(|(at, _)| (now - *at).num_milliseconds() as f64 / 1000.0)
                        .filter(|s| *s > 0.0);
                    let rate = IntakeRate {
                        window_seconds: seconds,
                        batches_per_second: seconds
                            .zip(previous_intake.as_ref())
                            .map(|(s, (_, p))| totals.batches.saturating_sub(p.batches) as f64 / s),
                        bytes_per_second: seconds.zip(previous_intake.as_ref()).map(
                            |(s, (_, p))| {
                                totals.received_bytes.saturating_sub(p.received_bytes) as f64 / s
                            },
                        ),
                        totals: totals.clone(),
                    };
                    previous_intake = Some((now, totals));
                    rate
                })
                .ok_or("intake_telemetry_unavailable");
            if let Ok(mut cache) = self.cache.write() {
                cache.sampled_at = Some(now);
                cache.model.record(model, now);
                cache.pools.record(Ok(pools), now);
                cache.intake.record(intake, now);
                if cache.history.len() == 60 {
                    cache.history.pop_front();
                }
                cache.history.push_back(ResourcePoint {
                    at: now,
                    rss_bytes: resources
                        .rss_bytes
                        .error
                        .is_none()
                        .then_some(resources.rss_bytes.value)
                        .flatten(),
                    cpu_cores: resources
                        .cpu_cores
                        .error
                        .is_none()
                        .then_some(resources.cpu_cores.value)
                        .flatten(),
                });
                cache.resources = Some(resources);
            }
            if tick.is_multiple_of(3) {
                let result = tokio::select! {
                    _ = shutdown.cancelled() => break,
                    result = tokio::time::timeout(Duration::from_secs(2), self.tasks.operations_counts()) => result,
                };
                if let Ok(mut cache) = self.cache.write() {
                    cache.tasks.record(
                        result
                            .ok()
                            .and_then(Result::ok)
                            .ok_or("task_counts_unavailable"),
                        Utc::now(),
                    );
                }
            }
            if let Some(storage) = &self.storage {
                tokio::select! { _ = shutdown.cancelled() => break, _ = storage.sample_next() => {} }
            }
            tick = tick.wrapping_add(1);
        }
    }
}
