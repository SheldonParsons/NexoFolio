use super::{LlmClient, LlmError};
use nexofolio_curate_contracts::telemetry::{Latency, ModelSnapshot, ModelTelemetry};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Telemetry {
    pub http: AtomicUsize,
    cancelled: AtomicU64,
    measured: Mutex<Measured>,
}
#[derive(Default)]
struct Measured {
    completed: u64,
    failures: BTreeMap<String, u64>,
    queue: VecDeque<u64>,
    request: VecDeque<u64>,
}
pub(super) struct Attempt<'a> {
    telemetry: &'a Telemetry,
    finished: bool,
}
impl Telemetry {
    pub fn attempt(&self) -> Attempt<'_> {
        Attempt {
            telemetry: self,
            finished: false,
        }
    }
}
impl Attempt<'_> {
    pub fn finish(mut self, queue_ms: u64, request_ms: Option<u64>, error: Option<&LlmError>) {
        self.finished = true;
        if let Ok(mut state) = self.telemetry.measured.lock() {
            state.completed = state.completed.saturating_add(1);
            if let Some(e) = error {
                *state
                    .failures
                    .entry(e.classification().code().into())
                    .or_default() += 1;
            }
            push(&mut state.queue, queue_ms);
            if let Some(ms) = request_ms {
                push(&mut state.request, ms);
            }
        }
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.telemetry.cancelled.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub(super) struct Gauge<'a>(&'a AtomicUsize);
impl<'a> Gauge<'a> {
    pub fn enter(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}
impl Drop for Gauge<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
fn push(values: &mut VecDeque<u64>, value: u64) {
    if values.len() == 256 {
        values.pop_front();
    }
    values.push_back(value);
}
fn latency(values: &VecDeque<u64>) -> Latency {
    let mut sorted: Vec<_> = values.iter().copied().collect();
    sorted.sort_unstable();
    Latency {
        samples: sorted.len(),
        mean_ms: (!sorted.is_empty())
            .then(|| sorted.iter().map(|v| *v as f64).sum::<f64>() / sorted.len() as f64),
        p95_ms: sorted
            .get((sorted.len() * 95).div_ceil(100).saturating_sub(1))
            .copied(),
    }
}
impl ModelTelemetry for LlmClient {
    fn model_snapshot(&self) -> Option<ModelSnapshot> {
        let (
            permits_in_use,
            waiting_for_permit,
            current_limit,
            configured_limit,
            cooldown_remaining_ms,
        ) = self.slots.snapshot()?;
        let state = self.telemetry.measured.lock().ok()?;
        Some(ModelSnapshot {
            http_in_flight: self.telemetry.http.load(Ordering::Relaxed),
            permits_in_use,
            waiting_for_permit,
            current_limit,
            configured_limit,
            cooldown_remaining_ms,
            completed: state.completed,
            cancelled: self.telemetry.cancelled.load(Ordering::Relaxed),
            failures: state.failures.clone(),
            queue: latency(&state.queue),
            request: latency(&state.request),
        })
    }
}
