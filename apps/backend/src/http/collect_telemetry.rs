//! Aggregate completed HTTP intake attempts, including refusals. No body is retained.
use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Default, Serialize)]
pub(crate) struct IntakeSnapshot {
    pub in_flight: usize,
    pub peak_in_flight: usize,
    pub admission_refused: u64,
    pub client_busy: u64,
    pub batches: u64,
    pub accepted_batches: u64,
    pub received_bytes: u64,
    pub unreadable_bodies: u64,
    pub http_429: u64,
    pub http_503: u64,
    pub latency_samples: usize,
    pub mean_receive_ms: Option<f64>,
    pub fetcher_local_queue: Option<u64>,
}
#[derive(Default)]
pub(crate) struct IntakeTelemetry(Mutex<(IntakeSnapshot, VecDeque<u64>)>);
impl IntakeTelemetry {
    pub fn in_flight(&self, count: usize) {
        if let Ok(mut state) = self.0.lock() {
            state.0.in_flight = count;
            state.0.peak_in_flight = state.0.peak_in_flight.max(count);
        }
    }
    pub fn admission_refused(&self, client_busy: bool) {
        if let Ok(mut state) = self.0.lock() {
            state.0.admission_refused += 1;
            state.0.client_busy += u64::from(client_busy);
        }
    }
    pub fn observe(&self, bytes: Option<usize>, status: u16, elapsed_ms: u64) {
        if let Ok(mut state) = self.0.lock() {
            state.0.batches += 1;
            state.0.accepted_batches += u64::from((200..300).contains(&status));
            state.0.received_bytes += bytes.unwrap_or(0) as u64;
            state.0.unreadable_bodies += u64::from(bytes.is_none());
            state.0.http_429 += u64::from(status == 429);
            state.0.http_503 += u64::from(status == 503);
            if state.1.len() == 256 {
                state.1.pop_front();
            }
            state.1.push_back(elapsed_ms);
        }
    }
    pub fn snapshot(&self) -> Option<IntakeSnapshot> {
        let state = self.0.lock().ok()?;
        let mut out = state.0.clone();
        out.latency_samples = state.1.len();
        out.mean_receive_ms = (!state.1.is_empty())
            .then(|| state.1.iter().map(|v| *v as f64).sum::<f64>() / state.1.len() as f64);
        Some(out)
    }
}
