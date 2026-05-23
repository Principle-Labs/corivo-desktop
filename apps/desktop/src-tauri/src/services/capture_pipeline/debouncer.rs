//! Per-surface debouncer (Littlebird's `ResettableTimer`).
//!
//! Events arrive faster than we want to capture (a single SPA route
//! change can fire half a dozen `TitleChanged` + `LayoutChanged`
//! notifications back-to-back). The debouncer absorbs that burst:
//!
//! - First event for a surface starts a sleep.
//! - Each subsequent event aborts the in-flight sleep and starts a new
//!   one — the timer "resets".
//! - When the sleep finally completes uninterrupted, we emit a single
//!   `SnapshotJob` for that surface.
//!
//! The "surface" key today is the foreground bundle id (what NSWorkspace
//! gives us). When AX events come online we'll widen the key to
//! `(bundle_id, window_id_or_title_hash)` so a multi-window app
//! doesn't share a debounce slot across windows.
//!
//! High-priority triggers (Manual, SafetyNet) bypass the debouncer
//! and emit immediately — there's no upstream burst to absorb.

use std::{collections::HashMap, time::Duration};

use tokio::{sync::mpsc, task::JoinHandle, time::sleep};

use crate::domain::snapshot_envelope::Trigger;

/// One scheduled snapshot the coordinator wants the job runner to do.
#[derive(Debug, Clone)]
pub struct SnapshotJob {
    pub trigger: Trigger,
    /// Surface key the debouncer used. Only useful for tracing today;
    /// once `event_log` lands it'll go in the row.
    pub surface_key: String,
}

/// Default debounce windows by trigger. Numbers are starting points
/// from the design spec — wire them through config when we have data
/// from a week of dogfood.
pub fn default_debounce(trigger: Trigger) -> Duration {
    match trigger {
        // App-activation already represents a settled state — capture
        // immediately. The user is looking at the new window now.
        Trigger::FocusChange => Duration::from_millis(0),
        // Focused-window changes inside the active app are similarly
        // settled — the user already finished switching.
        Trigger::FocusedWindowChanged => Duration::from_millis(0),
        // Title changes can fire many times during an SPA route swap
        // (each progressive render bumps the title). Hold off briefly
        // so we capture the settled state.
        Trigger::TitleChanged => Duration::from_millis(250),
        // Manual / quick_ask never debounce.
        Trigger::Manual | Trigger::QuickAsk => Duration::from_millis(0),
    }
}

/// Per-surface resettable timer. Owned by the coordinator — not Send
/// across tasks; the coordinator keeps it on its own task.
pub struct Debouncer {
    handles: HashMap<String, JoinHandle<()>>,
    job_tx: mpsc::Sender<SnapshotJob>,
}

impl Debouncer {
    pub fn new(job_tx: mpsc::Sender<SnapshotJob>) -> Self {
        Self {
            handles: HashMap::new(),
            job_tx,
        }
    }

    /// Schedule a job for `surface_key`, replacing any in-flight timer
    /// on the same key. If `delay` is zero we send synchronously and
    /// skip the timer — keeps front-app changes feeling instant.
    pub fn fire(&mut self, surface_key: String, job: SnapshotJob, delay: Duration) {
        // Drop the old handle first so its sleep aborts. `abort()` on
        // a finished task is a no-op, so we don't need to check.
        if let Some(h) = self.handles.remove(&surface_key) {
            h.abort();
        }

        if delay.is_zero() {
            // Try the non-blocking send; coordinator overflow is logged
            // upstream. Don't await on the cold path — we're inside a
            // single-task event loop.
            let job_tx = self.job_tx.clone();
            let key_for_log = surface_key.clone();
            tokio::spawn(async move {
                if let Err(error) = job_tx.send(job).await {
                    tracing::warn!(
                        ?error,
                        surface_key = %key_for_log,
                        "debouncer.job_send_failed"
                    );
                }
            });
            return;
        }

        let job_tx = self.job_tx.clone();
        let key_for_task = surface_key.clone();
        let handle = tokio::spawn(async move {
            sleep(delay).await;
            if let Err(error) = job_tx.send(job).await {
                tracing::warn!(
                    ?error,
                    surface_key = %key_for_task,
                    "debouncer.job_send_failed"
                );
            }
        });
        self.handles.insert(surface_key, handle);
    }

    /// Cancel every pending timer. Used on shutdown.
    pub fn drain(&mut self) {
        for (_, h) in self.handles.drain() {
            h.abort();
        }
    }
}

impl Drop for Debouncer {
    fn drop(&mut self) {
        self.drain();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn zero_delay_sends_immediately() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut deb = Debouncer::new(tx);
        deb.fire(
            "k".into(),
            SnapshotJob {
                trigger: Trigger::FocusChange,
                surface_key: "k".into(),
            },
            Duration::ZERO,
        );
        let job = timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("recv timeout")
            .expect("channel closed");
        assert_eq!(job.trigger, Trigger::FocusChange);
        assert_eq!(job.surface_key, "k");
    }

    #[tokio::test]
    async fn second_fire_replaces_first_within_window() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut deb = Debouncer::new(tx);
        // First fire schedules a slow job.
        deb.fire(
            "k".into(),
            SnapshotJob {
                trigger: Trigger::FocusChange,
                surface_key: "first".into(),
            },
            Duration::from_millis(80),
        );
        // Second fire BEFORE the first fires — the first is canceled,
        // the second starts a new short timer.
        deb.fire(
            "k".into(),
            SnapshotJob {
                trigger: Trigger::FocusChange,
                surface_key: "second".into(),
            },
            Duration::from_millis(20),
        );
        let job = timeout(Duration::from_millis(120), rx.recv())
            .await
            .expect("recv timeout")
            .expect("channel closed");
        // Only the second job should arrive; the first was aborted.
        assert_eq!(job.surface_key, "second");
        // No further messages.
        assert!(timeout(Duration::from_millis(120), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn drain_aborts_in_flight_timers() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut deb = Debouncer::new(tx);
        deb.fire(
            "k".into(),
            SnapshotJob {
                trigger: Trigger::FocusChange,
                surface_key: "k".into(),
            },
            Duration::from_millis(80),
        );
        deb.drain();
        // Nothing should arrive.
        assert!(timeout(Duration::from_millis(120), rx.recv())
            .await
            .is_err());
    }
}
