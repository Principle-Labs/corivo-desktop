//! Developer-mode commands. Surfaced from the hidden "Developer" tab in
//! Settings (unlocked by triple-clicking the dialog title). The tab itself
//! is the only gate — these commands are safe to call from any caller
//! that already has IPC access, since they're either read-only or do
//! exactly what the user-facing button promises.
//!
//! Production DevTools requires the `devtools` feature on `tauri` in
//! `Cargo.toml`; without it `WebviewWindow::open_devtools` is a no-op
//! in release builds.

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// Open the WebView inspector for a named Tauri window
/// (`main` / `quick-ask` / `notification-overlay`). Returns an error if
/// the label doesn't resolve, so the UI can show a meaningful message
/// instead of silently failing — useful when a developer is checking
/// whether a particular window is actually alive.
#[tauri::command]
pub async fn dev_open_devtools(app: AppHandle, label: String) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("Window not found: {label}"))?;
    window.open_devtools();
    Ok(())
}

/// Reveal the app data directory in Finder so the developer can poke at
/// `corivo.sqlite`, `config.json`, `captures/`, and `logs/` directly.
/// Mirrors `memory::open_background_task_logs_dir` — same opener plugin,
/// same `app_data_dir()` resolution, just one level higher.
#[tauri::command]
pub async fn dev_reveal_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app_data_dir unavailable: {error}"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("open failed: {error}"))?;
    Ok(())
}
