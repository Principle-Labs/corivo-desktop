//! Workflow-completion notification dispatch (v1700).
//!
//! When a scheduled-workflow run finishes, this module emits a single
//! `workflow:completed` Tauri event globally. Two windows listen:
//!
//! 1. `main` — invalidates React Query caches so the `/workflows` page
//!    repaints with the fresh `last_run_at` / `last_status`.
//!
//! 2. `notification-overlay` — renders the right-corner toast that
//!    surfaces the run to the user.
//!
//! v1700 cleanup: removed the `tauri-plugin-notification` macOS banner
//! path (the overlay window is the only outward surface now) and the
//! per-policy `dispatch_started` event (the "立即运行" button drives
//! its own local spinner via React Query's `isPending`).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Wry};

/// Wire payload of the `workflow:completed` event. Both windows
/// subscribe to the same shape.
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

/// Emit `workflow:completed` globally. Doesn't return errors — a
/// failed emit is telemetry-only; the audit row in `workflow_runs`
/// is the source of truth either way.
pub fn dispatch(
    app: &AppHandle<Wry>,
    name: &str,
    slug: &str,
    run_id: &str,
    thread_id: &str,
    summary: &str,
    success: bool,
) {
    let payload = WorkflowCompletedEvent {
        slug: slug.to_string(),
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        name: name.to_string(),
        summary: summary.to_string(),
        success,
    };
    if let Err(error) = app.emit(WorkflowCompletedEvent::EVENT, payload) {
        tracing::warn!(
            slug,
            ?error,
            "scheduled_workflow.notify_emit_failed"
        );
    }
}
