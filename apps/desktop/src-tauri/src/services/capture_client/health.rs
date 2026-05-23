//! Helper liveness tracker.
//!
//! The helper emits a `heartbeat` message every 5s. This module records
//! the last-seen timestamp; the read loop consults [`HealthMonitor::mark_alive`]
//! on each heartbeat, and consumers can read [`HealthMonitor::status`] to
//! check whether the helper has gone silent.
//!
//! Phase 0 keeps this minimal — no auto-restart, no restart-rate limiter.
//! Those land in a later phase along with the production lifecycle
//! integration in `lib.rs`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperHealthStatus {
    /// Heartbeat received within the alive window.
    Alive,
    /// No heartbeat received yet; helper might still be in handshake or
    /// idle but hasn't emitted its first heartbeat.
    Pending,
    /// Last heartbeat was more than `crashed_threshold` ago; helper is
    /// presumed crashed / wedged.
    Crashed,
}

#[derive(Clone)]
pub struct HealthMonitor {
    inner: Arc<Mutex<Inner>>,
    crashed_threshold: Duration,
}

#[derive(Default)]
struct Inner {
    last_alive: Option<Instant>,
}

impl HealthMonitor {
    /// `crashed_threshold` is how long without a heartbeat before we mark
    /// the helper crashed. Spec default: 15s (3× the 5s heartbeat cadence).
    pub fn new(crashed_threshold: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            crashed_threshold,
        }
    }

    pub fn with_default_threshold() -> Self {
        Self::new(Duration::from_secs(15))
    }

    /// Mark a heartbeat received now.
    pub fn mark_alive(&self) {
        self.inner.lock().expect("health mutex poisoned").last_alive = Some(Instant::now());
    }

    /// Read the current liveness status. Useful for the public
    /// `CaptureClient::health()` accessor and tests.
    pub fn status(&self) -> HelperHealthStatus {
        self.status_at(Instant::now())
    }

    /// Same as [`Self::status`] but parameterized on the reference time
    /// (test hook).
    pub fn status_at(&self, now: Instant) -> HelperHealthStatus {
        let inner = self.inner.lock().expect("health mutex poisoned");
        match inner.last_alive {
            None => HelperHealthStatus::Pending,
            Some(t) if now.duration_since(t) <= self.crashed_threshold => HelperHealthStatus::Alive,
            Some(_) => HelperHealthStatus::Crashed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_before_first_heartbeat() {
        let m = HealthMonitor::with_default_threshold();
        assert_eq!(m.status(), HelperHealthStatus::Pending);
    }

    #[test]
    fn alive_after_mark_alive() {
        let m = HealthMonitor::with_default_threshold();
        m.mark_alive();
        assert_eq!(m.status(), HelperHealthStatus::Alive);
    }

    #[test]
    fn crashed_when_last_heartbeat_exceeds_threshold() {
        let m = HealthMonitor::new(Duration::from_millis(50));
        m.mark_alive();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(m.status(), HelperHealthStatus::Crashed);
    }

    #[test]
    fn refreshes_alive_on_subsequent_heartbeat() {
        let m = HealthMonitor::new(Duration::from_millis(50));
        m.mark_alive();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(m.status(), HelperHealthStatus::Crashed);
        m.mark_alive();
        assert_eq!(m.status(), HelperHealthStatus::Alive);
    }
}
