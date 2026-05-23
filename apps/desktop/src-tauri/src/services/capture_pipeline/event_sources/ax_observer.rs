//! AX event subscription, fronting the helper's `ax.subscribe` /
//! `ax.unsubscribe` RPCs.
//!
//! **Phase 7 migration**: this module used to own its own CFRunLoop
//! thread + AXObserver via `accessibility-sys`. Both now live in the
//! capture helper sidecar
//! (`packages/desktop-helpers/macos/.../AX/AXObserverThread.swift`). The
//! Rust side becomes a thin client: subscribe per-pid via the helper,
//! translate `HelperEvent::Ax*` into the capture pipeline's existing
//! `Event::FocusedWindow` / `Event::TitleChanged` so downstream
//! consumers (the pipeline's `process_event` loop) don't need to
//! change.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tokio::sync::mpsc;

use super::super::event_bus::Event;
use crate::services::capture_client;
use crate::services::capture_client::{AxNotification, HelperEvent};

/// Same public surface the pre-Phase-7 thread had — the rest of the
/// capture pipeline doesn't need to know it now sits on top of an IPC
/// subscription.
pub struct AXObserverThread {
    state: Arc<Mutex<State>>,
    /// Tokio task that pulls helper events and forwards Ax* into `bus`.
    forwarder: tauri::async_runtime::JoinHandle<()>,
    /// `JoinHandle` slot kept for API parity with the legacy struct;
    /// always `None` now (no kernel thread under us anymore).
    _legacy_join: Option<JoinHandle<()>>,
}

struct State {
    current_pid: Option<i32>,
}

impl AXObserverThread {
    pub fn spawn(bus: mpsc::Sender<Event>) -> Self {
        let state = Arc::new(Mutex::new(State { current_pid: None }));

        // Subscribe to the broadcast bus immediately so we don't miss
        // events that fire before our task spins up.
        let mut rx = match capture_client::global::try_get() {
            Some(client) => client.subscribe_events(),
            None => {
                tracing::warn!("ax_observer.helper_unavailable_at_spawn");
                let (_tx, rx) = tokio::sync::broadcast::channel(1);
                rx
            }
        };

        let state_for_task = state.clone();
        let forwarder = tauri::async_runtime::spawn(async move {
            loop {
                let evt = match rx.recv().await {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, "ax_observer.event_lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let translated = match evt {
                    HelperEvent::AxFocusedWindowChanged { pid, .. } => {
                        Some((pid, Event::FocusedWindow))
                    }
                    HelperEvent::AxTitleChanged { pid, .. } => Some((pid, Event::TitleChanged)),
                    _ => None,
                };
                let Some((pid, evt)) = translated else {
                    continue;
                };
                // Drop events for stale pids (helper may emit one
                // straggler after we retarget; the pipeline only wants
                // events for the active subscription).
                let active = state_for_task.lock().ok().and_then(|g| g.current_pid);
                if active != Some(pid) {
                    tracing::trace!(?active, ?pid, "ax_observer.event_pid_mismatch_dropped");
                    continue;
                }
                if let Err(error) = bus.try_send(evt) {
                    tracing::warn!(?error, "ax_observer.bus_send_failed");
                }
            }
        });

        Self {
            state,
            forwarder,
            _legacy_join: None,
        }
    }

    /// Re-target the AX subscription to a new pid. Idempotent.
    pub fn set_target_pid(&self, pid: i32) {
        let prev = match self.state.lock() {
            Ok(mut g) => {
                let prev = g.current_pid;
                g.current_pid = Some(pid);
                prev
            }
            Err(poisoned) => {
                let mut g = poisoned.into_inner();
                let prev = g.current_pid;
                g.current_pid = Some(pid);
                prev
            }
        };
        if prev == Some(pid) {
            return;
        }
        tracing::info!(?prev, new_pid = pid, "ax_observer.retarget");
        let Some(client) = capture_client::global::try_get() else {
            tracing::warn!("ax_observer.set_target_pid.helper_unavailable");
            return;
        };
        let notifications = vec![AxNotification::FocusedWindow, AxNotification::Title];
        tauri::async_runtime::spawn(async move {
            if let Err(error) = client.subscribe_ax(pid, &notifications).await {
                tracing::warn!(?error, pid, "ax_observer.subscribe_ax_failed");
            }
        });
    }

    /// Cloneable closure that re-targets the AX subscription. Used by
    /// the foreground monitor's NSWorkspace block to drive AX retargeting
    /// on app activation without holding `&AXObserverThread`.
    pub fn pid_setter(&self) -> impl Fn(i32) + Send + Sync + 'static {
        let state = self.state.clone();
        move |pid: i32| {
            let prev = match state.lock() {
                Ok(mut g) => {
                    let prev = g.current_pid;
                    g.current_pid = Some(pid);
                    prev
                }
                Err(poisoned) => {
                    let mut g = poisoned.into_inner();
                    let prev = g.current_pid;
                    g.current_pid = Some(pid);
                    prev
                }
            };
            if prev == Some(pid) {
                return;
            }
            let Some(client) = capture_client::global::try_get() else {
                tracing::warn!("ax_observer.pid_setter.helper_unavailable");
                return;
            };
            let notifications = vec![AxNotification::FocusedWindow, AxNotification::Title];
            tauri::async_runtime::spawn(async move {
                if let Err(error) = client.subscribe_ax(pid, &notifications).await {
                    tracing::warn!(?error, pid, "ax_observer.pid_setter.subscribe_failed");
                }
            });
        }
    }

    /// Cooperatively tear down the subscription + forwarder task.
    pub fn shutdown(self) {
        let pid = match self.state.lock() {
            Ok(g) => g.current_pid,
            Err(poisoned) => poisoned.into_inner().current_pid,
        };
        if let (Some(client), Some(pid)) = (capture_client::global::try_get(), pid) {
            tauri::async_runtime::spawn(async move {
                let _ = client.unsubscribe_ax(pid).await;
            });
        }
        self.forwarder.abort();
    }
}

impl Drop for AXObserverThread {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}
