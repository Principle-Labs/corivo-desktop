//! Quick Ask Tauri command surface (spec §十一).
//!
//! Quick Ask shares the same chat backend as `/ask` (one shared
//! `exec_agent_send` running claude CLI + corivo MCP), so this module
//! only owns the things that *are* Quick-Ask specific:
//!
//!   - `hotkey_status` — read-only view of the double-tap-⌥ binding
//!     state for the Settings UI. The binding itself lives in
//!     `services/double_tap_hotkey.rs`; rebinding is not supported in
//!     v1 because `RegisterEventHotKey` won't bind a bare modifier.
//!   - `quick_ask_summon` / `quick_ask_hide` / `quick_ask_set_height`
//!     — open / close / resize the floating window.
//!   - `quick_ask_capture_focus` — run capture_pipeline.invoke_quick_ask
//!     and return a fresh thread + the FocusContext to seed the panel.
//!   - `quick_ask_open_in_app` — bring the main window forward and
//!     navigate `/ask` to a specific thread (the "在 App 中查看" affordance).
//!
//! The send turn itself goes through `exec_agent_send` with an
//! optional `focus_context` payload — Quick Ask supplies it, `/ask`
//! doesn't. There's no `quick_ask_send` and no pin / promote concept
//! anymore: every Quick Ask thread persists like any other and shows
//! up in the `/ask` sidebar by default.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    commands::config::AppState,
    domain::focus_context::FocusContext,
    services::hotkey::HotkeyStatus,
    services::macos_system_surface::{
        apply_macos_system_surface_mode, MacOSSystemSurfaceMode, DEFAULT_MAIN_WINDOW_LABEL,
    },
    services::quick_ask_window::{
        apply_quick_ask_overlay_window_mode, hide_quick_ask, show_quick_ask,
    },
};

/// `quick-ask` is the dedicated overlay window label. The window is
/// declared in tauri.conf.json (Batch 6) and pre-created at boot.
pub const QUICK_ASK_WINDOW_LABEL: &str = "quick-ask";

/// Window-scoped event the main app listens for to deep-link `/ask`
/// to a specific thread. Quick Ask emits it via `quick_ask_open_in_app`.
const ASK_OPEN_THREAD_EVENT: &str = "ask:open-thread";

#[tauri::command]
pub async fn quick_ask_summon(app: AppHandle) -> Result<(), String> {
    // All quick-ask window mutations must happen on the macOS main
    // thread — `to_panel`, `set_level`, `makeKeyWindow` all assert
    // main-thread, and the existing `quick_ask_set_height` /
    // `show_quick_ask_window` paths follow the same rule.
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        if let Some(window) = app_for_task.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
            apply_quick_ask_overlay_window_mode(&window);
            show_quick_ask(&window);
        }
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn quick_ask_hide(app: AppHandle) -> Result<(), String> {
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        if let Some(window) = app_for_task.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
            hide_quick_ask(&window);
        }
    })
    .map_err(|e| e.to_string())
}

/// Window sizing constants. Height is driven by the frontend: a
/// `ResizeObserver` watches the rendered shell and pushes the natural
/// pixel height back through `quick_ask_set_height` so the window
/// always tracks the React content (no dead vibrancy strip below the
/// input bar). The min / max bounds keep the panel from shrinking to
/// nothing on first paint or growing past the screen on a long
/// transcript.
pub const QUICK_ASK_WIDTH: f64 = 640.0;
pub const QUICK_ASK_INITIAL_HEIGHT: f64 = 140.0;
pub const QUICK_ASK_MIN_HEIGHT: f64 = 100.0;
pub const QUICK_ASK_MAX_HEIGHT: f64 = 800.0;

#[tauri::command]
pub async fn quick_ask_set_height(app: AppHandle, height: f64) -> Result<(), String> {
    let app_for_task = app.clone();
    let clamped = height.clamp(QUICK_ASK_MIN_HEIGHT, QUICK_ASK_MAX_HEIGHT);
    // Window mutation must run on the macOS main thread — same
    // constraint that `show_quick_ask_window` honors via `run_on_main`
    // in lib.rs.
    app.run_on_main_thread(move || {
        if let Some(window) = app_for_task.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
            let _ = window.set_size(tauri::LogicalSize::new(QUICK_ASK_WIDTH, clamped));
        }
    })
    .map_err(|e| e.to_string())
}

/// Diagnostic-only: the React FocusCard reports which app + state it
/// is currently rendering. Folded into the main tracing stream so the
/// Rust-side log shows the full chain (NSWorkspace activate → emit →
/// React render) without having to crack open WebView devtools.
#[tauri::command]
pub async fn quick_ask_log_display(app: Option<String>, source: String) -> Result<(), String> {
    tracing::info!(
        app = app.as_deref().unwrap_or("(none)"),
        source = %source,
        "quick_ask.focus_card.displaying"
    );
    Ok(())
}

#[tauri::command]
pub async fn quick_ask_capture_focus(state: State<'_, AppState>) -> Result<FocusContext, String> {
    let pipeline = state
        .capture_pipeline
        .as_ref()
        .ok_or_else(|| "capture_pipeline not initialized".to_string())?
        .clone();
    // Type-to-create: no `chat_threads` row is minted here. The thread
    // materializes on the first send via `chat_thread_create` from the
    // frontend (see `useChatStream::createThreadIfMissing`), same flow
    // `/ask` uses. This keeps the sidebar free of empty rows for every
    // hotkey press the user didn't follow up on.
    pipeline.invoke_quick_ask().await.map_err(|e| e.to_string())
}

/// Bring the main window forward and tell `/ask` to focus a specific
/// thread. Backs the "在 App 中查看" button in the Quick Ask panel:
/// hide the overlay, raise the main window (same surface promotion
/// the dock icon does), and emit `ask:open-thread` so AppBoot can
/// router-navigate to `/ask?threadId=…`.
#[tauri::command]
pub async fn quick_ask_open_in_app(app: AppHandle, thread_id: String) -> Result<(), String> {
    // Hide the overlay first so the main window comes forward without
    // the panel chrome lingering.
    if let Some(window) = app.get_webview_window(QUICK_ASK_WINDOW_LABEL) {
        let _ = window.hide();
    }

    // Promote back to a regular dock-icon app so the main window can
    // actually take focus. Mirrors the tray-click + hotkey paths.
    let _ = apply_macos_system_surface_mode(&app, MacOSSystemSurfaceMode::RegularForeground);

    let app_for_main = app.clone();
    let thread_for_main = thread_id.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if let Some(window) = app_for_main.get_webview_window(DEFAULT_MAIN_WINDOW_LABEL) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        if let Err(error) = app_for_main.emit_to(
            DEFAULT_MAIN_WINDOW_LABEL,
            ASK_OPEN_THREAD_EVENT,
            &thread_for_main,
        ) {
            tracing::warn!(?error, "quick_ask.emit_open_thread_failed");
        }
    }) {
        return Err(format!("dispatch to main thread failed: {error}"));
    }

    Ok(())
}

#[tauri::command]
pub async fn hotkey_status(state: State<'_, AppState>) -> Result<HotkeyStatus, String> {
    let svc = state
        .hotkey_service
        .as_ref()
        .ok_or_else(|| "hotkey service not initialized".to_string())?
        .clone();
    Ok(svc.current().await)
}
