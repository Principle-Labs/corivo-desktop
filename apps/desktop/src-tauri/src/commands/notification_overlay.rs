//! IPC commands for the notification-overlay window (v1700).
//!
//! Three surfaces:
//!
//! * `notification_overlay_show` — re-position the window on the
//!   active screen, apply panel mode (idempotent), then order it
//!   front. Called by the React app every time a new
//!   `workflow:completed` event arrives.
//!
//! * `notification_overlay_dismiss` — order the panel out. Called
//!   from the React app on auto-hide / × click / after navigating
//!   to a run.
//!
//! * `notification_overlay_open_run` — hide the overlay, bring the
//!   main window forward, and emit `workflow:open-run` so the main
//!   app router can deep-link into `/ask` for that run. Wires the
//!   "click toast → see the run" affordance.

use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::services::notification_overlay_window::{
    apply_notification_overlay_window_mode, hide_overlay, position_top_right, show_overlay,
};

pub const NOTIFICATION_OVERLAY_WINDOW_LABEL: &str = "notification-overlay";

/// Event the main window already subscribes to (originally for Quick
/// Ask's "在 App 中查看"). Reuse it so the toast click works through
/// the same deep-link path — payload is the bare thread id.
const ASK_OPEN_THREAD_EVENT: &str = "ask:open-thread";

#[tauri::command]
pub async fn notification_overlay_show(app: AppHandle<Wry>) -> Result<(), String> {
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        let Some(window) = app_for_task.get_webview_window(NOTIFICATION_OVERLAY_WINDOW_LABEL) else {
            tracing::warn!("notification_overlay.show.window_missing");
            return;
        };
        apply_notification_overlay_window_mode(&window);
        position_top_right(&window);
        show_overlay(&window);
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn notification_overlay_dismiss(app: AppHandle<Wry>) -> Result<(), String> {
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        if let Some(window) = app_for_task.get_webview_window(NOTIFICATION_OVERLAY_WINDOW_LABEL) {
            hide_overlay(&window);
        }
    })
    .map_err(|e| e.to_string())
}

/// Hide the toast, bring the main window forward, and emit
/// `ask:open-thread` with the run's thread id. The main window's
/// app-boot listener routes `/ask?threadId=…` from there, reusing
/// the Quick Ask "在 App 中查看" plumbing.
#[tauri::command]
pub async fn notification_overlay_open_run(
    app: AppHandle<Wry>,
    thread_id: String,
) -> Result<(), String> {
    use crate::services::macos_system_surface::{
        apply_macos_system_surface_mode, MacOSSystemSurfaceMode, DEFAULT_MAIN_WINDOW_LABEL,
    };

    if thread_id.trim().is_empty() {
        // The run had no chat thread (failure path where the runner
        // bailed before creating one). Surface the dismiss + dock
        // promotion anyway so the click still feels responsive, but
        // don't navigate to a stale route.
        if let Some(window) = app.get_webview_window(NOTIFICATION_OVERLAY_WINDOW_LABEL) {
            let _ = window.hide();
        }
        let _ = apply_macos_system_surface_mode(&app, MacOSSystemSurfaceMode::RegularForeground);
        return Ok(());
    }

    // Hide the overlay first so the main-window raise doesn't fight a
    // floating panel for the foreground slot.
    if let Some(window) = app.get_webview_window(NOTIFICATION_OVERLAY_WINDOW_LABEL) {
        let _ = window.hide();
    }

    // Promote back to a regular dock-icon app so the main window can
    // actually take focus. Mirrors the quick_ask_open_in_app pattern.
    let _ = apply_macos_system_surface_mode(&app, MacOSSystemSurfaceMode::RegularForeground);

    let app_for_main = app.clone();
    let thread_for_main = thread_id.clone();
    app.run_on_main_thread(move || {
        if let Some(window) = app_for_main.get_webview_window(DEFAULT_MAIN_WINDOW_LABEL) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        if let Err(error) =
            app_for_main.emit_to(DEFAULT_MAIN_WINDOW_LABEL, ASK_OPEN_THREAD_EVENT, &thread_for_main)
        {
            tracing::warn!(?error, "notification_overlay.emit_open_thread_failed");
        }
    })
    .map_err(|e| format!("dispatch to main thread failed: {e}"))
}
