//! Trailing debouncer used by the preferences store.
//!
//! The debouncer holds the most recently scheduled callback and fires
//! it `delay` after the **last** `schedule` call. Repeated `schedule`
//! calls within the window reset the timer so a continuous slider
//! drag results in exactly one disk write at the end.
//!
//! Implementation notes:
//!
//! - The debouncer spawns a single worker thread at construction
//!   time. The worker waits on a `Condvar` for either the deadline
//!   or a cancellation signal.
//! - `cancel` discards any pending callback without firing it.
//! - `drain_for_test` runs the pending callback synchronously so
//!   tests can assert behaviour without sleeping.
//!
//! The implementation is intentionally minimal — no async runtime, no
//! generics over the callback type. The preferences store is the only
//! caller.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

/// Trailing debouncer. Cheap to clone — every field is behind an
/// `Arc`.
#[derive(Debug, Clone)]
pub struct Debouncer {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: Mutex<State>,
    cv: Condvar,
    /// Default delay used when the caller doesn't override it.
    delay: Duration,
}

/// Mutable state shared with the worker thread. `pending` is not
/// `Debug`-able because closures don't implement `Debug`; the manual
/// impl skips it.
struct State {
    /// Most recently scheduled callback. Replaced on every
    /// `schedule` call so the timer always fires with the latest
    /// value.
    pending: Option<Box<dyn FnOnce() + Send + 'static>>,
    /// Deadline the worker thread waits for. Reset on every
    /// `schedule` call.
    deadline: Option<Instant>,
    /// Monotonic counter incremented on every `cancel` call. Used
    /// by the worker to detect when a callback it observed has been
    /// invalidated by a subsequent cancel/schedule.
    cancel_generation: u64,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("deadline", &self.deadline)
            .field("cancel_generation", &self.cancel_generation)
            .finish_non_exhaustive()
    }
}

impl Debouncer {
    /// Build a new debouncer with the given delay. Spawns the
    /// worker thread eagerly so the first `schedule` call has
    /// someone to wake.
    pub fn new(delay: Duration) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                pending: None,
                deadline: None,
                cancel_generation: 0,
            }),
            cv: Condvar::new(),
            delay,
        });
        let worker_inner = inner.clone();
        thread::Builder::new()
            .name("pixelgrab-prefs-debouncer".to_string())
            .spawn(move || worker_loop(worker_inner))
            .expect("spawn preferences debouncer thread");
        Self { inner }
    }

    /// Schedule `callback` to fire after the trailing delay. The
    /// delay is measured from the most recent call, so repeated calls
    /// within the window only fire the latest callback.
    pub fn schedule<F>(&self, callback: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let deadline = Instant::now() + self.inner.delay;
        let mut state = self.inner.state.lock();
        state.pending = Some(Box::new(callback));
        state.deadline = Some(deadline);
        state.cancel_generation = state.cancel_generation.wrapping_add(1);
        self.inner.cv.notify_all();
    }

    /// Cancel any pending callback without firing it.
    pub fn cancel(&self) {
        let mut state = self.inner.state.lock();
        state.pending = None;
        state.deadline = None;
        state.cancel_generation = state.cancel_generation.wrapping_add(1);
        self.inner.cv.notify_all();
    }

    /// Test-only: run the pending callback synchronously if there is
    /// one. Returns `true` when a callback ran. The pending state is
    /// cleared whether or not the callback ran.
    pub fn drain_for_test(&self) -> bool {
        let mut state = self.inner.state.lock();
        let cb = state.pending.take();
        state.deadline = None;
        if let Some(cb) = cb {
            cb();
            true
        } else {
            false
        }
    }
}

fn worker_loop(inner: Arc<Inner>) {
    loop {
        // Wait until there is a deadline to honour.
        let deadline = {
            let mut state = inner.state.lock();
            loop {
                if let Some(d) = state.deadline {
                    break d;
                }
                inner.cv.wait(&mut state);
            }
        };
        // Wait until the deadline (or a re-schedule / cancel wakes us).
        let callback = {
            let mut state = inner.state.lock();
            let now = Instant::now();
            if deadline > now {
                let timeout = deadline - now;
                inner.cv.wait_for(&mut state, timeout);
            }
            // If the deadline was reset (a newer schedule came in)
            // or removed (a cancel), this iteration is invalidated —
            // wait for the next one.
            if state.deadline != Some(deadline) {
                None
            } else {
                state.deadline = None;
                state.pending.take()
            }
        };
        if let Some(cb) = callback {
            cb();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn callback_fires_after_delay() {
        let debouncer = Debouncer::new(Duration::from_millis(20));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        debouncer.schedule(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repeated_schedule_only_fires_latest() {
        let debouncer = Debouncer::new(Duration::from_millis(40));
        let counter = Arc::new(AtomicUsize::new(0));
        for i in 0..5 {
            let c = counter.clone();
            debouncer.schedule(move || {
                c.fetch_add(i, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(10));
        }
        // Wait long enough for the trailing callback to fire.
        std::thread::sleep(Duration::from_millis(100));
        // Only the last (i=4) increment ran.
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn cancel_discards_pending() {
        let debouncer = Debouncer::new(Duration::from_millis(30));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        debouncer.schedule(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        debouncer.cancel();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn drain_for_test_runs_immediately() {
        let debouncer = Debouncer::new(Duration::from_secs(60));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        debouncer.schedule(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let ran = debouncer.drain_for_test();
        assert!(ran);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        // Second drain is a no-op.
        assert!(!debouncer.drain_for_test());
    }

    #[test]
    fn worker_thread_is_alive_after_first_schedule() {
        // The worker is spawned eagerly, but it must continue
        // servicing subsequent schedule calls. Wait, schedule again,
        // and confirm the second callback fires.
        let debouncer = Debouncer::new(Duration::from_millis(20));
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let c = counter.clone();
            debouncer.schedule(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(60));
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
