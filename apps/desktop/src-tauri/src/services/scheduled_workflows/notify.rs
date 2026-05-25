//! Two-channel notification dispatch for scheduled-workflow runs
//! (v1432).
//!
//! Each fire goes through this module once. We emit:
//!
//! 1. **macOS banner** via `tauri-plugin-notification`. Surfaces even
//!    when the app is backgrounded. Title = workflow name, body =
//!    truncated summary. Best-effort — TCC permission may be
//!    unauthorized; we log and continue.
//!
//! 2. **`workflow:completed` Tauri event** on the `main` window. The
//!    frontend listens, fires a sonner toast for in-app visibility,
//!    and invalidates the React Query cache so `next_run_at` /
//!    `last_status` refresh immediately.
//!
//! The sidebar "Corivo 提议" section is the persistent surface — it
//! reads `workflow_runs` directly; we don't need to push to it here.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Wry};
use tauri_plugin_notification::NotificationExt;

/// Wire payload of the `workflow:completed` event. Mirrors the fields
/// the frontend toast + invalidator both need.
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowCompletedEvent {
    pub slug: String,
    pub run_id: String,
    pub thread_id: String,
    pub name: String,
    pub summary: String,
    pub success: bool,
}

impl WorkflowCompletedEvent {
    pub const EVENT: &'static str = "workflow:completed";
}

/// v1432.1 — emitted right before the sidecar spawns. Frontend uses
/// it to flip a Zustand store entry to "in-flight" so the list row
/// shows a spinner and the toast becomes a persistent
/// "正在运行..." loading toast instead of the transient queued
/// notification.
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowStartedEvent {
    pub slug: String,
    pub run_id: String,
    pub name: String,
}

impl WorkflowStartedEvent {
    pub const EVENT: &'static str = "workflow:started";
}

/// Fire the started event. Doesn't return errors — failures here are
/// telemetry-only, the run keeps going regardless.
pub fn dispatch_started(
    app: &AppHandle<Wry>,
    name: &str,
    slug: &str,
    run_id: &str,
) {
    let payload = WorkflowStartedEvent {
        slug: slug.to_string(),
        run_id: run_id.to_string(),
        name: name.to_string(),
    };
    if let Err(error) = app.emit_to("main", WorkflowStartedEvent::EVENT, payload) {
        tracing::warn!(
            slug,
            ?error,
            "scheduled_workflow.notify_started_emit_failed"
        );
    }
}

/// Fire both channels. Doesn't return errors — notification failures
/// must never block the run-recording path. Tracing captures anything
/// that goes wrong.
pub fn dispatch(
    app: &AppHandle<Wry>,
    name: &str,
    slug: &str,
    run_id: &str,
    thread_id: &str,
    summary: &str,
    success: bool,
) {
    // macOS banner. The `notification` plugin honors the user's TCC
    // grant; if denied, `show()` returns an `Err` we just log.
    let banner_title = if success {
        name.to_string()
    } else {
        format!("{name} · 运行失败")
    };
    if let Err(error) = app
        .notification()
        .builder()
        .title(banner_title)
        .body(summary)
        .show()
    {
        tracing::warn!(
            slug,
            ?error,
            "scheduled_workflow.notify_banner_failed"
        );
    }

    let payload = WorkflowCompletedEvent {
        slug: slug.to_string(),
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        name: name.to_string(),
        summary: summary.to_string(),
        success,
    };
    // Use `emit_to("main", ...)` rather than the global `emit` so the
    // Quick Ask overlay window doesn't get a toast it can't render.
    if let Err(error) = app.emit_to("main", WorkflowCompletedEvent::EVENT, payload) {
        tracing::warn!(
            slug,
            ?error,
            "scheduled_workflow.notify_emit_failed"
        );
    }
}
