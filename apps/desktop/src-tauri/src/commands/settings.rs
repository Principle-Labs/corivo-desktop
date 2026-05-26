use std::process::Command;

use chrono::Duration;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::db::time::now_utc;

#[derive(Debug, serde::Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub os_version: String,
    pub app_version: String,
    pub tauri_version: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DataPaths {
    pub app_data_dir: String,
    pub config_path: String,
    pub captures_dir: String,
}

#[tauri::command]
pub async fn set_autostart_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart
            .enable()
            .map_err(|error| format!("Enabling launch-at-login failed: {error}"))
    } else {
        autostart
            .disable()
            .map_err(|error| format!("Disabling launch-at-login failed: {error}"))
    }
}

#[tauri::command]
pub async fn get_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| format!("Reading launch-at-login state failed: {error}"))
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    Ok(SystemInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        os_version: sys_info::os_release().unwrap_or_else(|_| "unknown".to_string()),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        tauri_version: "2".to_string(),
    })
}

#[tauri::command]
pub async fn restart_as_administrator(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe()
            .map_err(|error| format!("Could not resolve current executable: {error}"))?;
        Command::new("powershell")
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-Command")
            .arg("Start-Process -FilePath $env:CORIVO_ELEVATE_EXE -Verb RunAs")
            .env("CORIVO_ELEVATE_EXE", exe)
            .spawn()
            .map_err(|error| format!("Could not request administrator restart: {error}"))?;
        app.exit(0);
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err("Administrator restart is only available on Windows".to_string())
    }
}

#[tauri::command]
pub async fn clear_all_screenshots(state: State<'_, AppState>) -> Result<i64, String> {
    if let Some(pipeline) = &state.capture_pipeline {
        if pipeline.is_running() {
            return Err("Stop capture before clearing screenshots".to_string());
        }
    }

    let sessions = state
        .capture_store
        .list_sessions()
        .await
        .map_err(String::from)?;

    let mut total = 0;
    for session in sessions {
        state
            .capture_store
            .delete_session(&session.id)
            .await
            .map_err(String::from)?;
        total += session.screenshot_count;
    }

    Ok(total)
}

#[derive(Debug, serde::Serialize)]
pub struct HardDeleteSummary {
    pub frames_before: i64,
    pub sessions_deleted: i64,
    pub screenshots_deleted: i64,
}

/// Outcome of a granular "delete the last N seconds" wipe. Mirrors the
/// shape of `HardDeleteSummary` enough that a single UI surface can
/// render either response with one set of strings.
#[derive(Debug, Default, Clone, serde::Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct DeleteRangeReport {
    /// Number of frame rows actually removed.
    pub frames_deleted: i64,
    /// Number of screenshot files cleaned up from disk. Always
    /// `<= frames_deleted` (a frame without a screenshot still counts
    /// as a row removed; a missing file on disk does not).
    pub screenshots_deleted: i64,
    /// RFC3339 UTC instant the cutoff resolved to. Useful for the
    /// frontend to confirm to the user "you deleted everything since X".
    pub cutoff: String,
}

/// Drop every frame captured in the trailing `seconds_back` seconds plus
/// their on-disk screenshots. Intended for the "delete last 5 min /
/// 15 min / custom…" entries in the privacy popover. Capped at one year
/// so a runaway IPC caller can't accidentally request a value that
/// translates to a far-past cutoff (which is fine semantically but feels
/// wrong as the "last N seconds" idiom).
///
/// Unlike `data_hard_delete` this does NOT stop the capture pipeline —
/// the cutoff is in the past, so any frame the pipeline ingests after
/// the call falls outside the window. SQLite + the filesystem are
/// touched in that order (rows first, files second) so a mid-run crash
/// leaves orphaned files at worst, never orphaned rows.
#[tauri::command]
pub async fn data_delete_range(
    state: State<'_, AppState>,
    seconds_back: u64,
) -> Result<DeleteRangeReport, String> {
    const MAX_SECONDS: u64 = 365 * 24 * 60 * 60;
    let clamped = seconds_back.min(MAX_SECONDS);
    let cutoff = now_utc() - Duration::seconds(clamped as i64);
    let cutoff_iso = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let Some(repo) = state.frames_repo.as_ref() else {
        return Ok(DeleteRangeReport {
            frames_deleted: 0,
            screenshots_deleted: 0,
            cutoff: cutoff_iso,
        });
    };

    let screenshot_paths = repo.delete_since(cutoff).await.map_err(String::from)?;
    let frames_deleted = screenshot_paths.len() as i64;

    // File deletion is best-effort. A frame row with no on-disk file
    // (the OCR fallback writes a screenshot but the AX path doesn't —
    // `screenshot_path` is then null and never lands in the path list)
    // is fine; a row with a path that no longer exists is also fine.
    // We log non-NotFound failures but never propagate them — losing
    // the row is what privacy guarantees, the file cleanup is hygiene.
    let captures_root = state.capture_store.captures_root().clone();
    let mut screenshots_deleted = 0i64;
    for relative in &screenshot_paths {
        let candidate = std::path::Path::new(relative);
        let abs_path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            captures_root.join(candidate)
        };
        match tokio::fs::remove_file(&abs_path).await {
            Ok(()) => {
                screenshots_deleted += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Already gone — nothing to do.
            }
            Err(error) => {
                tracing::warn!(
                    path = %abs_path.display(),
                    error = %error,
                    "data.delete_range: screenshot cleanup failed"
                );
            }
        }
    }

    tracing::info!(
        seconds_back = clamped,
        frames_deleted,
        screenshots_deleted,
        cutoff = %cutoff_iso,
        "data.delete_range_complete"
    );

    Ok(DeleteRangeReport {
        frames_deleted,
        screenshots_deleted,
        cutoff: cutoff_iso,
    })
}

/// Drop every observation Corivo holds: SQLite tables (frames, embeddings,
/// chat threads, …) plus all screenshot files on disk.
///
/// Stops the capture pipeline before wiping so an in-flight tick doesn't
/// race against the schema rebuild. Caller is expected to hard-confirm
/// in the UI; there is no undo.
#[tauri::command]
pub async fn data_hard_delete(state: State<'_, AppState>) -> Result<HardDeleteSummary, String> {
    let pipeline = state.capture_pipeline.clone();
    if let Some(pipeline) = pipeline.as_ref() {
        if pipeline.is_running() {
            pipeline.stop().await.map_err(String::from)?;
        }
    }

    let frames_before = if let Some(repo) = state.frames_repo.as_ref() {
        repo.count().await.map_err(String::from)?
    } else {
        0
    };

    // Wipe the on-disk store first — purge_legacy_and_apply_new is
    // transactional inside SQLite, but the screenshot dir lives outside
    // that transaction. Doing files first means a mid-run crash leaves
    // the user with an empty captures/ but a stale DB; the next boot's
    // migration check would not trigger a rebuild on its own, so we
    // clean up the schema next.
    let sessions = state
        .capture_store
        .list_sessions()
        .await
        .map_err(String::from)?;
    let mut sessions_deleted = 0i64;
    let mut screenshots_deleted = 0i64;
    for session in sessions {
        state
            .capture_store
            .delete_session(&session.id)
            .await
            .map_err(String::from)?;
        sessions_deleted += 1;
        screenshots_deleted += session.screenshot_count;
    }

    state.db.wipe_and_rebuild().await.map_err(String::from)?;

    // Clear cached session ids in long-lived services so the next
    // capture / Quick Ask invoke lazy-creates fresh ones instead of
    // pointing at the now-missing session dirs.
    if let Some(pipeline) = pipeline.as_ref() {
        pipeline.reset_quick_ask_cache().await;
    }

    tracing::info!(
        frames_before,
        sessions_deleted,
        screenshots_deleted,
        "data.hard_delete_complete"
    );

    Ok(HardDeleteSummary {
        frames_before,
        sessions_deleted,
        screenshots_deleted,
    })
}

#[cfg(target_os = "macos")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// 只读检查屏幕录制权限，不会弹出系统授权弹窗。
#[tauri::command]
pub async fn check_screen_recording_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(unsafe { CGPreflightScreenCaptureAccess() })
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(true)
    }
}

/// 只读检查 Accessibility 权限。AX 是 enhancement（spec §六：Screen
/// Recording 是 hard requirement，AX 不开则降级 OCR-only），
/// onboarding 这一屏可跳过。
#[tauri::command]
pub async fn check_ax_permission() -> Result<bool, String> {
    Ok(crate::services::extractor::ax_extractor::is_process_trusted().await)
}

/// 打开 Privacy & Security → Accessibility 面板（per ax-ocr-spec.md Q7）。
#[tauri::command]
pub async fn open_ax_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|error| format!("Could not open System Settings: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// 主动请求屏幕录制权限，首次调用会弹出 macOS 系统授权弹窗。
/// 返回值是调用时的权限状态，注意 macOS 授权后需要重启 app 才生效，
/// 所以即使用户点了允许，这个函数也可能返回 false。
#[tauri::command]
pub async fn request_screen_recording_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(unsafe { CGRequestScreenCaptureAccess() })
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(true)
    }
}

#[tauri::command]
pub async fn open_system_settings_privacy() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn()
            .map_err(|error| format!("Could not open System Settings: {error}"))?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[tauri::command]
pub async fn open_captures_directory(state: State<'_, AppState>) -> Result<(), String> {
    open_path(state.capture_store.captures_root())
}

#[tauri::command]
pub async fn get_data_paths(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DataPaths, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve app data dir: {error}"))?;

    Ok(DataPaths {
        app_data_dir: app_data_dir.to_string_lossy().to_string(),
        config_path: app_data_dir
            .join("config.json")
            .to_string_lossy()
            .to_string(),
        captures_dir: state
            .capture_store
            .captures_root()
            .to_string_lossy()
            .to_string(),
    })
}

fn open_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open directory: {error}"))?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open directory: {error}"))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open directory: {error}"))?;
        return Ok(());
    }
}
