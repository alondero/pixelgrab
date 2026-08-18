//! Controllable monotonic + wall clock for tests.

use std::sync::Mutex;

/// A clock whose value is controlled by the test. Wall-clock millis are
/// simulated by a starting epoch plus an explicit offset.
#[derive(Debug)]
pub struct ControllableClock {
    inner: Mutex<ClockState>,
}

#[derive(Debug, Clone)]
struct ClockState {
    epoch_ms: i64,
    elapsed_ms: i64,
}

impl ControllableClock {
    /// Create a new clock starting at the Unix epoch with zero elapsed time.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ClockState {
                epoch_ms: 1_700_000_000_000,
                elapsed_ms: 0,
            }),
        }
    }

    /// Return the current wall-clock millis (epoch + elapsed).
    pub fn now_ms(&self) -> i64 {
        let state = self.inner.lock().expect("clock poisoned");
        state.epoch_ms + state.elapsed_ms
    }

    /// Return the monotonic elapsed millis.
    pub fn elapsed_ms(&self) -> i64 {
        let state = self.inner.lock().expect("clock poisoned");
        state.elapsed_ms
    }

    /// Advance the clock by `delta_ms`.
    pub fn advance(&self, delta_ms: i64) {
        let mut state = self.inner.lock().expect("clock poisoned");
        state.elapsed_ms = state.elapsed_ms.saturating_add(delta_ms);
    }

    /// Reset the clock to zero elapsed time.
    pub fn reset(&self) {
        let mut state = self.inner.lock().expect("clock poisoned");
        state.elapsed_ms = 0;
    }
}

impl Default for ControllableClock {
    fn default() -> Self {
        Self::new()
    }
}
