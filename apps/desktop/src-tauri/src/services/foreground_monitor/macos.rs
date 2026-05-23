//! macOS implementation of [`ForegroundAppMonitor`].
//!
//! **Phase 7 migration**: this used to register an `NSWorkspace`
//! observer block directly via `objc2-app-kit`. The block was the
//! authoritative source of "frontmost app changed" for both the Quick
//! Ask overlay and the capture pipeline. The observer now lives in the
//! capture helper sidecar (`Foreground/ForegroundHandlers.swift`); the
//! Rust side just subscribes to the helper's broadcast event bus and
//! fans the events out to:
//!
//!   1. **Quick Ask overlay** — always on. Emits `capture:focus-activated`
//!      to the `quick-ask` webview the moment NSWorkspace tells the
//!      helper a new app is frontmost, so the FocusCard re-aims with
//!      no user prompt latency.
//!
//!   2. **Capture pipeline (optional)** — pushes `Event::AppActivated`
//!      onto the capture event bus and retargets the AX subscription to
//!      the new pid. Wired through [`Self::subscribe`] when the
//!      pipeline starts and torn down via [`Self::unsubscribe`] when it
//!      stops.
//!
//! Same-app re-activation dedup (Electron tooltip dismissals etc.)
//! happens on the Rust side using `last_bundle`, exactly as the old
//! NSWorkspace impl did.

use std::sync::{Arc, Mutex};

use tauri::{async_runtime::JoinHandle, AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::error::{CorivoError, Result};
use crate::services::capture_client;
use crate::services::capture_client::HelperEvent;
use crate::services::capture_pipeline::event_bus::Event;

pub const FOCUS_ACTIVATED_EVENT: &str = "capture:focus-activated";
const QUICK_ASK_LABEL: &str = "quick-ask";

pub type PidHook = Arc<dyn Fn(i32) + Send + Sync + 'static>;

struct CaptureSubscriber {
    bus: mpsc::Sender<Event>,
    pid_hook: PidHook,
}

struct Inner {
    last_bundle: Option<String>,
    subscriber: Option<CaptureSubscriber>,
}

pub struct ForegroundAppMonitor {
    inner: Arc<Mutex<Inner>>,
    /// Background task that pulls events from the helper bus and fans
    /// them out. Aborted on Drop so it doesn't outlive the monitor.
    task: Mutex<Option<JoinHandle<()>>>,
}

impl ForegroundAppMonitor {
    pub fn install(notify_tauri: AppHandle) -> Result<Arc<Self>> {
        let client = capture_client::global::get().map_err(|e| {
            CorivoError::Internal(format!(
                "foreground_monitor: capture helper unavailable: {e}"
            ))
        })?;

        let inner = Arc::new(Mutex::new(Inner {
            last_bundle: None,
            subscriber: None,
        }));
        let our_pid = std::process::id() as i32;

        // Start the helper-side observer if it isn't already running.
        // Best-effort — if this fails the monitor still installs (the
        // event task just won't see anything until subscribe succeeds).
        let client_for_subscribe = client.clone();
        let subscribe_task = tauri::async_runtime::spawn(async move {
            if let Err(error) = client_for_subscribe.subscribe_foreground().await {
                tracing::warn!(?error, "foreground_monitor.helper_subscribe_failed");
            }
        });
        // Detach — we don't await; subscribe is fire-and-forget and any
        // failure is logged.
        std::mem::drop(subscribe_task);

        // Spawn the event-forwarding task.
        let mut rx = client.subscribe_events();
        let inner_for_task = inner.clone();
        let notify = notify_tauri.clone();
        let task = tauri::async_runtime::spawn(async move {
            loop {
                let evt = match rx.recv().await {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, "foreground_monitor.event_lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("foreground_monitor.event_bus_closed");
                        break;
                    }
                };
                if let HelperEvent::ForegroundAppActivated {
                    pid,
                    bundle_id,
                    app_name,
                    ..
                } = evt
                {
                    Self::dispatch(&inner_for_task, &notify, our_pid, pid, bundle_id, app_name);
                }
            }
        });

        tracing::info!("foreground_monitor.installed");
        Ok(Arc::new(Self {
            inner,
            task: Mutex::new(Some(task)),
        }))
    }

    fn dispatch(
        inner: &Arc<Mutex<Inner>>,
        notify_tauri: &AppHandle,
        our_pid: i32,
        pid: Option<i32>,
        bundle_id: Option<String>,
        app_name: Option<String>,
    ) {
        let (deduped, from, subscriber) = {
            let mut guard = match inner.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let prior = guard.last_bundle.clone();
            let deduped = prior == bundle_id;
            if !deduped {
                guard.last_bundle = bundle_id.clone();
            }
            let subscriber = guard.subscriber.as_ref().map(|s| CaptureSubscriber {
                bus: s.bus.clone(),
                pid_hook: s.pid_hook.clone(),
            });
            (deduped, prior, subscriber)
        };

        if deduped {
            // Even on a same-app reactivation the pid may have changed
            // (app relaunch); keep the AX target current.
            if let (Some(sub), Some(pid)) = (&subscriber, pid) {
                (sub.pid_hook)(pid);
            }
            return;
        }

        // Self-skip: drop the entire activation when it points at
        // Corivo. Two checks because either signal alone can lie —
        // pid==our_pid catches the common case, and the bundle prefix
        // covers helper sub-bundles or builds where the helper reports
        // a pid we can't match (e.g. a wrapper process). Once we decide
        // it's "us", we don't emit FOCUS_ACTIVATED, we don't push
        // AppActivated onto the capture bus, and we don't retarget the
        // AX subscription — none of those are useful for our own UI.
        let is_self_pid = pid == Some(our_pid);
        let is_self_bundle = bundle_id
            .as_deref()
            .map(crate::services::exclusion::matches_self)
            .unwrap_or(false);
        if is_self_pid || is_self_bundle {
            tracing::info!(
                ?bundle_id,
                ?pid,
                is_self_pid,
                is_self_bundle,
                "foreground_monitor.skipped_self"
            );
            return;
        }

        let payload = serde_json::json!({
            "app_bundle_id": bundle_id,
            "app_name": app_name,
            "pid": pid,
        });
        match notify_tauri.get_webview_window(QUICK_ASK_LABEL) {
            Some(window) => {
                if let Err(error) = window.emit(FOCUS_ACTIVATED_EVENT, &payload) {
                    tracing::warn!(?error, "foreground_monitor.emit_failed");
                }
            }
            None => tracing::warn!("foreground_monitor.quick_ask_window_missing"),
        }

        if let Some(sub) = subscriber {
            let event = Event::AppActivated {
                from,
                to: bundle_id,
            };
            if let Err(error) = sub.bus.try_send(event) {
                tracing::warn!(?error, "foreground_monitor.bus_send_failed");
            }
            if let Some(pid) = pid {
                (sub.pid_hook)(pid);
            }
        }
    }

    pub fn subscribe(
        &self,
        bus: mpsc::Sender<Event>,
        pid_hook: Box<dyn Fn(i32) + Send + Sync + 'static>,
    ) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.subscriber = Some(CaptureSubscriber {
            bus,
            pid_hook: Arc::from(pid_hook),
        });
        tracing::info!("foreground_monitor.subscribed");
    }

    pub fn unsubscribe(&self) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.subscriber = None;
        tracing::info!("foreground_monitor.unsubscribed");
    }

    /// Push a synthetic `AppActivated` for the current frontmost app,
    /// asking the helper for the live state. Used by the capture
    /// pipeline's `start` to seed the first frame without waiting for
    /// the next NSWorkspace activation.
    pub fn emit_initial_focus(bus: &mpsc::Sender<Event>) -> Result<()> {
        let bus = bus.clone();
        let client = capture_client::global::try_get();
        tauri::async_runtime::spawn(async move {
            let bundle_id = if let Some(client) = client {
                match client.current_foreground().await {
                    Ok(app) => app.bundle_id,
                    Err(error) => {
                        tracing::warn!(?error, "foreground_monitor.initial_focus_helper_failed");
                        None
                    }
                }
            } else {
                None
            };
            if let Err(error) = bus.try_send(Event::AppActivated {
                from: None,
                to: bundle_id,
            }) {
                tracing::warn!(?error, "foreground_monitor.initial_focus_send_failed");
            }
        });
        Ok(())
    }
}

impl Drop for ForegroundAppMonitor {
    fn drop(&mut self) {
        let task = match self.task.lock() {
            Ok(mut g) => g.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(task) = task {
            task.abort();
        }
        // Best-effort helper unsubscribe.
        if let Some(client) = capture_client::global::try_get() {
            tauri::async_runtime::spawn(async move {
                let _ = client.unsubscribe_foreground().await;
            });
        }
        tracing::info!("foreground_monitor.dropped");
    }
}
