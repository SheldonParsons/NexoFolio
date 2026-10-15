//! Pure test-store transaction gate. No executor or IO dependency in contracts.
use std::{
    sync::{Arc, Mutex},
    task::{Poll, Waker},
};

#[derive(Default)]
pub(super) struct Gate {
    locked: bool,
    waiters: Vec<Waker>,
}
pub(super) struct Guard(Arc<Mutex<Gate>>);
pub(super) async fn acquire(gate: Arc<Mutex<Gate>>) -> Guard {
    std::future::poll_fn(|cx| {
        let mut state = gate.lock().expect("test transaction gate");
        if !state.locked {
            state.locked = true;
            Poll::Ready(Guard(gate.clone()))
        } else {
            if !state.waiters.iter().any(|w| w.will_wake(cx.waker())) {
                state.waiters.push(cx.waker().clone());
            }
            Poll::Pending
        }
    })
    .await
}
impl Drop for Guard {
    fn drop(&mut self) {
        let waiters = {
            let mut state = self.0.lock().expect("test transaction gate");
            state.locked = false;
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
    }
}
