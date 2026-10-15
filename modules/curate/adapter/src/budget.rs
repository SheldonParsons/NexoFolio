//! Single provider admission controller; elapsed/queue time never changes capacity.
use std::{sync::Mutex, time::Duration};
use tokio::{
    sync::{Semaphore, SemaphorePermit},
    time::Instant,
};
const COOLDOWN: Duration = Duration::from_secs(30);
struct State {
    ceiling: usize,
    in_flight: usize,
    not_before: Option<Instant>,
    limit: usize,
    debt: usize,
    successes: usize,
    generation: u64,
    changed_at: Option<Instant>,
    overloaded_at: Option<Instant>,
}
pub(super) struct Budget {
    slots: Semaphore,
    state: Mutex<State>,
    waiting: std::sync::atomic::AtomicUsize,
}
pub(super) struct Permit<'a> {
    permit: Option<SemaphorePermit<'a>>,
    budget: &'a Budget,
    generation: u64,
}
impl Budget {
    pub(super) fn new(limit: usize) -> Self {
        let limit = limit.clamp(1, 16);
        Self {
            slots: Semaphore::new(limit),
            waiting: std::sync::atomic::AtomicUsize::new(0),
            state: Mutex::new(State {
                ceiling: limit,
                in_flight: 0,
                not_before: None,
                limit,
                debt: 0,
                successes: 0,
                generation: 0,
                changed_at: None,
                overloaded_at: None,
            }),
        }
    }
    #[cfg(test)]
    pub(super) async fn acquire(&self) -> Result<Permit<'_>, super::LlmError> {
        self.acquire_inner(true).await
    }
    pub(super) async fn acquire_ready(&self) -> Result<Permit<'_>, super::LlmError> {
        self.acquire_inner(false).await
    }
    async fn acquire_inner(&self, wait_for_cooldown: bool) -> Result<Permit<'_>, super::LlmError> {
        loop {
            let deadline = self.state.lock().expect("model budget").not_before;
            if let Some(deadline) = deadline {
                if !wait_for_cooldown && deadline > Instant::now() {
                    return Err(super::LlmError::Cooldown {
                        retry_after_seconds: deadline
                            .saturating_duration_since(Instant::now())
                            .as_secs()
                            .saturating_add(1),
                    });
                }
                tokio::time::sleep_until(deadline).await;
            }
            let waiting = super::telemetry::Gauge::enter(&self.waiting);
            let permit = self
                .slots
                .acquire()
                .await
                .map_err(|_| super::LlmError::Transport("模型队列已关闭".into()))?;
            drop(waiting);
            let mut state = self.state.lock().expect("model budget");
            // Reclaim permits reserved by cancelled waiters before admitting HTTP IO.
            if state.debt > 0 {
                state.debt -= 1;
                permit.forget();
                continue;
            }
            if state.not_before.is_some_and(|t| t > Instant::now()) {
                drop(permit);
                continue;
            }
            state.in_flight += 1;
            return Ok(Permit {
                permit: Some(permit),
                budget: self,
                generation: state.generation,
            });
        }
    }

    pub(super) fn snapshot(&self) -> Option<(usize, usize, usize, usize, u64)> {
        let state = self.state.lock().ok()?;
        Some((
            state.in_flight,
            self.waiting.load(std::sync::atomic::Ordering::Relaxed),
            state.limit,
            state.ceiling,
            state.not_before.map_or(0, |t| {
                t.saturating_duration_since(Instant::now()).as_millis() as u64
            }),
        ))
    }

    fn observe(
        &self,
        generation: u64,
        success: bool,
        overloaded: bool,
        retry_after: Option<Duration>,
    ) {
        let mut state = self.state.lock().expect("model budget");
        if overloaded {
            let delay = retry_after.unwrap_or(COOLDOWN).max(COOLDOWN);
            let deadline = Instant::now() + delay;
            state.not_before = Some(state.not_before.map_or(deadline, |t| t.max(deadline)));
            state.overloaded_at = Some(Instant::now());
            tracing::info!(
                cooldown_seconds = delay.as_secs(),
                "shared model cooldown scheduled"
            );
        }
        // Requests admitted before the last adjustment cannot trigger another cut
        // or premature recovery. A burst of 429s is one overload event.
        if generation != state.generation {
            return;
        }
        let before = state.limit;
        let cooled = state.changed_at.is_none_or(|t| t.elapsed() >= COOLDOWN);
        let recovered = state.overloaded_at.is_none_or(|t| t.elapsed() >= COOLDOWN);
        if overloaded {
            state.successes = 0;
            state.overloaded_at = Some(Instant::now());
            if cooled {
                let target = (state.limit / 2).min(state.in_flight / 2).max(1);
                let remove = state.limit - target;
                state.debt += remove - self.slots.forget_permits(remove);
                state.limit = target;
                state.generation += 1;
                state.changed_at = Some(Instant::now());
            }
        } else if success {
            state.successes += 1;
            if cooled && recovered && state.successes >= state.limit && state.limit < state.ceiling
            {
                state.limit += 1;
                state.successes = 0;
                state.generation += 1;
                state.changed_at = Some(Instant::now());
                if state.debt > 0 {
                    state.debt -= 1;
                } else {
                    self.slots.add_permits(1);
                }
            }
        }
        if before != state.limit {
            tracing::info!(
                limit = state.limit,
                ceiling = state.ceiling,
                "shared model budget adjusted"
            );
        }
    }
}
impl Permit<'_> {
    pub(super) fn observe(&self, success: bool, overloaded: bool, retry_after: Option<Duration>) {
        self.budget
            .observe(self.generation, success, overloaded, retry_after);
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("model budget");
        state.in_flight -= 1;
        if state.debt > 0 {
            state.debt -= 1;
            self.permit.take().expect("owned permit").forget();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn a_burst_cuts_once_cancellation_repays_debt_and_success_recovers() {
        let budget = Budget::new(4);
        let a = budget.acquire().await.unwrap();
        let b = budget.acquire().await.unwrap();
        let c = budget.acquire().await.unwrap();
        let d = budget.acquire().await.unwrap();
        a.observe(false, true, None);
        b.observe(false, true, None);
        c.observe(false, true, None);
        assert_eq!(budget.state.lock().unwrap().limit, 2);
        drop(a);
        drop(b);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), budget.acquire())
                .await
                .is_err()
        );
        drop(c);
        drop(d);
        assert_eq!(budget.slots.available_permits(), 2);
        tokio::time::advance(COOLDOWN).await;
        for _ in 0..2 {
            budget.acquire().await.unwrap().observe(true, false, None);
        }
        assert_eq!(budget.slots.available_permits(), 3);
        tokio::time::advance(COOLDOWN).await;
        for _ in 0..3 {
            budget.acquire().await.unwrap().observe(true, false, None);
        }
        assert_eq!(budget.slots.available_permits(), 4);
    }
    #[tokio::test(start_paused = true)]
    async fn slow_and_failed_generations_do_not_reduce_provider_capacity() {
        let budget = Budget::new(8);
        for _ in 0..30 {
            let permit = budget.acquire().await.unwrap();
            tokio::time::advance(Duration::from_secs(150)).await;
            permit.observe(false, false, None);
        }
        assert_eq!(budget.slots.available_permits(), 8);
    }

    async fn overload_at_capacity(budget: &Budget) {
        let count = budget.state.lock().unwrap().limit;
        let mut permits = Vec::new();
        for _ in 0..count {
            permits.push(budget.acquire().await.unwrap());
        }
        permits[0].observe(false, true, None);
    }
    #[tokio::test(start_paused = true)]
    async fn continued_explicit_overload_can_reduce_to_one_but_recovers_after_success() {
        let budget = Budget::new(8);
        for expected in [4, 2, 1] {
            overload_at_capacity(&budget).await;
            assert_eq!(budget.slots.available_permits(), expected);
            tokio::time::advance(COOLDOWN).await;
        }
        budget.acquire().await.unwrap().observe(true, false, None);
        assert_eq!(budget.slots.available_permits(), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn four_description_calls_reduce_to_two_and_all_new_calls_respect_retry_after() {
        let budget = Budget::new(8);
        let mut permits = Vec::new();
        for _ in 0..4 {
            permits.push(budget.acquire().await.unwrap());
        }
        permits[0].observe(false, true, Some(Duration::from_secs(65)));
        permits[1].observe(false, true, None); // Same burst must not reduce again or shorten delay.
        drop(permits);
        assert_eq!(budget.slots.available_permits(), 2);
        let started = Instant::now();
        assert!(
            tokio::time::timeout(Duration::from_secs(64), budget.acquire())
                .await
                .is_err()
        );
        let permit = budget.acquire().await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(65));
        drop(permit);
        assert_eq!(budget.slots.available_permits(), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn cancelled_cooldown_waiter_does_not_occupy_a_slot() {
        let budget = Budget::new(4);
        overload_at_capacity(&budget).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(1), budget.acquire())
                .await
                .is_err()
        );
        assert_eq!(budget.slots.available_permits(), 2);
        assert_eq!(budget.state.lock().unwrap().in_flight, 0);
    }
    #[tokio::test(start_paused = true)]
    async fn cancelled_semaphore_reservations_cannot_bypass_the_reduced_capacity() {
        let budget = Budget::new(8);
        let mut active = Vec::new();
        for _ in 0..4 {
            active.push(budget.acquire().await.unwrap());
        }
        // Simulate queued acquire futures granted a permit but not yet polled into our wrapper.
        let mut reserved = Vec::new();
        for _ in 0..4 {
            reserved.push(budget.slots.acquire().await.unwrap());
        }
        active[0].observe(false, true, None);
        drop(active);
        drop(reserved);
        tokio::time::advance(COOLDOWN).await;
        let a = budget.acquire().await.unwrap();
        let b = budget.acquire().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), budget.acquire())
                .await
                .is_err()
        );
        drop(a);
        drop(b);
        assert_eq!(budget.slots.available_permits(), 2);
        assert_eq!(budget.state.lock().unwrap().debt, 0);
    }
}
