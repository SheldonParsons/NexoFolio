//! Non-waiting admission before body extraction. Only active clients occupy memory.
use super::collect_telemetry::IntakeTelemetry;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub(crate) const DEFAULT_IN_FLIGHT: usize = 4;
pub(crate) const CLIENT_HEADER: &str = "x-nexofolio-client-id";

pub(crate) struct Admission {
    limit: usize,
    active: Mutex<HashMap<Option<Uuid>, usize>>,
    telemetry: Arc<IntakeTelemetry>,
}

impl Admission {
    pub fn new(limit: usize, telemetry: Arc<IntakeTelemetry>) -> Arc<Self> {
        Arc::new(Self {
            limit,
            active: Mutex::new(HashMap::new()),
            telemetry,
        })
    }

    pub fn enter(self: &Arc<Self>, client: Option<Uuid>) -> Option<Permit> {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let total: usize = active.values().sum();
        // A single installation (or the shared legacy bucket) uses at most half
        // the slots. This is bounded opportunity, not an authentication boundary.
        let client_full = active.get(&client).copied().unwrap_or(0) >= self.limit.div_ceil(2);
        if total >= self.limit || client_full {
            self.telemetry.admission_refused(client_full);
            return None;
        }
        *active.entry(client).or_default() += 1;
        self.telemetry.in_flight(total + 1);
        Some(Permit {
            admission: self.clone(),
            client,
        })
    }
}

pub(crate) struct Permit {
    admission: Arc<Admission>,
    client: Option<Uuid>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut active = self
            .admission
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(count) = active.get_mut(&self.client) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.client);
            }
        }
        self.admission.telemetry.in_flight(active.values().sum());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_active_clients_and_shared_legacy_slots_release_on_drop() {
        let telemetry = Arc::new(IntakeTelemetry::default());
        let gate = Admission::new(4, telemetry.clone());
        let a = Some(Uuid::new_v4());
        let permits = [gate.enter(a).unwrap(), gate.enter(a).unwrap()];
        assert!(gate.enter(a).is_none());
        let legacy = gate.enter(None).unwrap();
        let other = gate.enter(Some(Uuid::new_v4())).unwrap();
        for _ in 0..2000 {
            assert!(gate.enter(Some(Uuid::new_v4())).is_none());
        }
        assert_eq!(gate.active.lock().unwrap().len(), 3);
        drop((permits, legacy, other));
        assert!(gate.active.lock().unwrap().is_empty());
        let snapshot = telemetry.snapshot().unwrap();
        assert_eq!((snapshot.in_flight, snapshot.peak_in_flight), (0, 4));
        assert_eq!(snapshot.admission_refused, 2001);
        assert!(gate.enter(None).is_some());
    }

    #[tokio::test]
    async fn cancelled_blocking_work_retains_its_permit_until_the_poll_finishes() {
        let gate = Admission::new(1, Arc::new(IntakeTelemetry::default()));
        let permit = gate.enter(None).unwrap();
        let (started_tx, started) = tokio::sync::oneshot::channel();
        let (finish, finished) = std::sync::mpsc::channel();
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            started_tx.send(()).unwrap();
            finished.recv().unwrap();
        });
        started.await.unwrap();
        work.abort();
        assert!(gate.enter(None).is_none());
        finish.send(()).unwrap();
        work.await.unwrap();
        assert!(gate.enter(None).is_some());
    }
}
