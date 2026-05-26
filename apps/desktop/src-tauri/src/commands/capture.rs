//! Capture-pipeline control surface.
//!
//!   - `capture_status`            — running / stopped + current session_id + paused_until
//!   - `capture_start`             — start the pipeline (no-op if already running)
//!   - `capture_stop`              — stop the pipeline
//!   - `capture_pause`             — pause screen reading for N seconds
//!                                   (also exposed via menubar tray menu)
//!   - `capture_resume`            — clear an active pause
//!   - `capture_request_snapshot`  — manual one-shot trigger
//!   - `capture_diagnostics`       — runtime health for the Settings privacy page
//!   - exclusion_*                 — bundle-id allowlist mgmt
//!
//! Capture is event-driven (NSWorkspace + AXObserver + IPC); knobs like
//! interval / batch / JPEG quality are not user-facing — they live as
//! defaults in `CaptureConfig` and are tweaked only by debug builds.

use serde::Serialize;
use tauri::State;

use crate::{
    commands::config::AppState,
    services::{
        capture_pipeline::CaptureStatus,
        exclusion::{
            default_blocked_bundles, self_bundle_prefixes, self_executable_names, ExclusionEngine,
        },
    },
};

/// Maximum pause window the menubar / IPC allows in one shot. Today
/// the tray exposes 1 hour and 1 day; this cap exists so a runaway
/// IPC caller can't paint themselves into a multi-week corner.
const MAX_PAUSE_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct CaptureStatusDto {
    pub phase: String,
    pub current_session_id: Option<String>,
    /// RFC3339 UTC instant when an active pause expires. `None` when
    /// capture is not paused — the frontend uses this to flip the
    /// sidebar status row between "已暂停" and "已暂停 · 还剩 X 分钟".
    pub paused_until: Option<String>,
}

impl From<CaptureStatus> for CaptureStatusDto {
    fn from(status: CaptureStatus) -> Self {
        Self {
            phase: status.phase.as_str().to_string(),
            current_session_id: status.current_session_id,
            paused_until: status
                .paused_until
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        }
    }
}

#[tauri::command]
pub async fn capture_status(state: State<'_, AppState>) -> Result<CaptureStatusDto, String> {
    let Some(pipeline) = &state.capture_pipeline else {
        return Ok(CaptureStatusDto {
            phase: "stopped".to_string(),
            current_session_id: None,
            paused_until: None,
        });
    };
    Ok(pipeline.status().await.into())
}

#[tauri::command]
pub async fn capture_start(state: State<'_, AppState>) -> Result<String, String> {
    let pipeline = state
        .capture_pipeline
        .as_ref()
        .ok_or_else(|| "capture pipeline not yet initialized".to_string())?
        .clone();
    pipeline.start().await.map_err(String::from)
}

#[tauri::command]
pub async fn capture_stop(state: State<'_, AppState>) -> Result<(), String> {
    let Some(pipeline) = &state.capture_pipeline else {
        return Ok(());
    };
    pipeline.stop().await.map_err(String::from)
}

/// Pause screen reading for `seconds` and persist the expiry timestamp
/// so the pause survives an app relaunch. The tray menu wires this to
/// 1 hour and 1 day; richer custom durations would just call the same
/// IPC.
#[tauri::command]
pub async fn capture_pause(
    state: State<'_, AppState>,
    seconds: i64,
) -> Result<CaptureStatusDto, String> {
    if seconds <= 0 {
        return Err("pause duration must be positive".to_string());
    }
    if seconds > MAX_PAUSE_SECONDS {
        return Err(format!(
            "pause duration too long (max {MAX_PAUSE_SECONDS}s)"
        ));
    }
    let pipeline = state
        .capture_pipeline
        .as_ref()
        .ok_or_else(|| "capture pipeline not yet initialized".to_string())?
        .clone();
    let until = chrono::Utc::now() + chrono::Duration::seconds(seconds);
    pipeline.pause_until(until).await.map_err(String::from)?;

    let mut persisted = state.config_service.get();
    persisted.app.capture_paused_until = Some(until);
    state
        .config_service
        .update(persisted)
        .map_err(String::from)?;

    Ok(pipeline.status().await.into())
}

/// Clear an active pause. No-op if capture isn't paused. If the
/// pipeline was running when pause began, it's restarted; otherwise
/// it stays stopped (matching the user's prior intent).
#[tauri::command]
pub async fn capture_resume(state: State<'_, AppState>) -> Result<CaptureStatusDto, String> {
    let pipeline = state
        .capture_pipeline
        .as_ref()
        .ok_or_else(|| "capture pipeline not yet initialized".to_string())?
        .clone();
    pipeline.resume().await.map_err(String::from)?;

    let mut persisted = state.config_service.get();
    if persisted.app.capture_paused_until.is_some() {
        persisted.app.capture_paused_until = None;
        state
            .config_service
            .update(persisted)
            .map_err(String::from)?;
    }

    Ok(pipeline.status().await.into())
}

/// Manual snapshot trigger (event driver only). Publishes a
/// `ManualSnapshot` event onto the bus; the coordinator turns it into
/// a `Trigger::Manual` frame. Returns an error if capture isn't
/// running in event mode.
#[tauri::command]
pub async fn capture_request_snapshot(
    state: State<'_, AppState>,
    reason: Option<String>,
) -> Result<(), String> {
    let pipeline = state
        .capture_pipeline
        .as_ref()
        .ok_or_else(|| "capture pipeline not yet initialized".to_string())?
        .clone();
    pipeline
        .request_snapshot(reason.unwrap_or_else(|| "ipc".to_string()))
        .await
        .map_err(|e| e.to_string())
}

/// Snapshot of capture's runtime health for the Settings → 隐私 page
/// and CLI debugging. Holds enough info to answer "why am I not
/// seeing events?" without diving into logs.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureDiagnosticDto {
    /// Pipeline phase ("running" / "stopped").
    pub phase: String,
    /// Active session id, if running.
    pub current_session_id: Option<String>,
    /// AX permission state. False here = AXObserver can't subscribe,
    /// `title_changed` / `focused_window_changed` won't fire, only
    /// `focus_change` (NSWorkspace) and `manual` events make it
    /// through. **Most common cause of "feels like a timer" symptoms.**
    pub ax_trusted: bool,
    /// Frame counts grouped by trigger over the last hour.
    pub recent_triggers: Vec<TriggerCountDto>,
    /// Most recent frame's captured_at (ISO-8601), if any frame exists.
    pub last_frame_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerCountDto {
    pub trigger: String,
    pub count: i64,
}

#[tauri::command]
pub async fn capture_diagnostic_status(
    state: State<'_, AppState>,
) -> Result<CaptureDiagnosticDto, String> {
    let phase;
    let current_session_id;
    if let Some(pipeline) = &state.capture_pipeline {
        let status = pipeline.status().await;
        phase = status.phase.as_str().to_string();
        current_session_id = status.current_session_id;
    } else {
        phase = "stopped".to_string();
        current_session_id = None;
    }

    let ax_trusted = crate::services::extractor::ax_extractor::is_process_trusted().await;

    let mut recent_triggers = Vec::new();
    let mut last_frame_at: Option<String> = None;
    if let Some(repo) = state.frames_repo.as_ref() {
        if let Ok(rows) = repo.trigger_counts_since_minutes_ago(60).await {
            recent_triggers = rows
                .into_iter()
                .map(|(trigger, count)| TriggerCountDto { trigger, count })
                .collect();
        }
        if let Ok(latest) = repo.most_recent_captured_at().await {
            last_frame_at =
                latest.map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        }
    }

    Ok(CaptureDiagnosticDto {
        phase,
        current_session_id,
        ax_trusted,
        recent_triggers,
        last_frame_at,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ExclusionEntry {
    pub bundle_id: String,
    /// "default" rows are baked into the binary and not removable;
    /// "user" rows live in `Config.exclusion.extra_app_bundle_ids`.
    pub source: String,
}

#[tauri::command]
pub async fn exclusion_list(state: State<'_, AppState>) -> Result<Vec<ExclusionEntry>, String> {
    let cfg = state.config_service.get();
    // Corivo's own bundle prefix(es) come first so the user sees that
    // self-capture is hard-blocked by default. The trailing `.*` is
    // purely a visual hint that the entry is a prefix match — the
    // engine matches `<prefix>` and `<prefix>.<anything>` regardless.
    let mut out: Vec<ExclusionEntry> = self_bundle_prefixes()
        .iter()
        .map(|prefix| ExclusionEntry {
            bundle_id: format!("{prefix}.*"),
            source: "default".to_string(),
        })
        .collect();
    out.extend(self_executable_names().iter().map(|exe| ExclusionEntry {
        bundle_id: (*exe).to_string(),
        source: "default".to_string(),
    }));
    out.extend(default_blocked_bundles().iter().map(|id| ExclusionEntry {
        bundle_id: (*id).to_string(),
        source: "default".to_string(),
    }));
    for bundle_id in cfg.exclusion.extra_app_bundle_ids.iter() {
        // Hide user rows that duplicate a default — keeps the list
        // tidy and avoids the user wondering why removing their entry
        // doesn't actually unblock the app. Both exact-match defaults
        // and the self-prefix engine rule shadow user adds.
        if default_blocked_bundles().iter().any(|d| *d == bundle_id) {
            continue;
        }
        if crate::services::exclusion::matches_self(bundle_id) {
            continue;
        }
        out.push(ExclusionEntry {
            bundle_id: bundle_id.clone(),
            source: "user".to_string(),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn exclusion_add(
    state: State<'_, AppState>,
    bundle_id: String,
) -> Result<Vec<ExclusionEntry>, String> {
    let trimmed = bundle_id.trim().to_string();
    if trimmed.is_empty() {
        return Err("bundle id cannot be empty".into());
    }
    if trimmed.len() > 200 {
        return Err("bundle id is too long".into());
    }

    let mut persisted = state.config_service.get();
    if !persisted
        .exclusion
        .extra_app_bundle_ids
        .iter()
        .any(|id| id == &trimmed)
    {
        persisted.exclusion.extra_app_bundle_ids.push(trimmed);
    }
    rebuild_pipeline_exclusion(&state, &persisted).await;
    state
        .config_service
        .update(persisted)
        .map_err(String::from)?;
    exclusion_list(state).await
}

#[tauri::command]
pub async fn exclusion_remove(
    state: State<'_, AppState>,
    bundle_id: String,
) -> Result<Vec<ExclusionEntry>, String> {
    let mut persisted = state.config_service.get();
    persisted
        .exclusion
        .extra_app_bundle_ids
        .retain(|id| id != &bundle_id);
    rebuild_pipeline_exclusion(&state, &persisted).await;
    state
        .config_service
        .update(persisted)
        .map_err(String::from)?;
    exclusion_list(state).await
}

async fn rebuild_pipeline_exclusion(
    state: &State<'_, AppState>,
    cfg: &crate::domain::config::Config,
) {
    if let Some(pipeline) = &state.capture_pipeline {
        let engine =
            ExclusionEngine::with_extras(cfg.exclusion.extra_app_bundle_ids.iter().cloned());
        pipeline.update_exclusion(engine).await;
    }
}

// ---- Website exclusion -----------------------------------------------------
//
// Companion to the app/bundle-id exclusion list above. Stored in
// `Config.exclusion.extra_website_patterns` as the raw user input
// (`notion.so` or `*.notion.so`); the engine in
// `services::website_exclusion` parses + validates on engine build.
//
// Today these commands only persist patterns to Config — the live
// capture pipeline doesn't consume the engine yet because
// `foreground::probe_current().url` is still `None`. Once URL
// extraction lights up, the pipeline picks up the engine without any
// Tauri-facing changes (see the TODO marker in
// `services/capture_pipeline/mod.rs`).

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct WebsiteExclusionEntry {
    /// Pattern as the user typed it, normalised to lowercase. Either an
    /// exact host (`notion.so`) or a wildcard subdomain match
    /// (`*.notion.so`).
    pub pattern: String,
    /// "user" today — there are no curated defaults. The field exists
    /// for parity with `ExclusionEntry.source` so the UI can use a single
    /// component for both lists; a future curated default list (banking
    /// portals, etc.) would surface here as "default".
    pub source: String,
}

#[tauri::command]
pub async fn website_exclusion_list(
    state: State<'_, AppState>,
) -> Result<Vec<WebsiteExclusionEntry>, String> {
    let cfg = state.config_service.get();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in cfg.exclusion.extra_website_patterns.iter() {
        // Parse-and-canonicalise so the UI always shows the normalised
        // form (`Notion.SO` and `notion.so` collapse to one row).
        let Some(parsed) = crate::services::website_exclusion::WebsitePattern::parse(raw) else {
            continue;
        };
        let display = parsed.display();
        if seen.insert(display.clone()) {
            out.push(WebsiteExclusionEntry {
                pattern: display,
                source: "user".to_string(),
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn website_exclusion_add(
    state: State<'_, AppState>,
    pattern: String,
) -> Result<Vec<WebsiteExclusionEntry>, String> {
    let parsed = crate::services::website_exclusion::WebsitePattern::parse(&pattern)
        .ok_or_else(|| "invalid website pattern".to_string())?;
    let canonical = parsed.display();
    if canonical.len() > 200 {
        return Err("website pattern is too long".into());
    }

    let mut persisted = state.config_service.get();
    if !persisted
        .exclusion
        .extra_website_patterns
        .iter()
        .filter_map(|s| crate::services::website_exclusion::WebsitePattern::parse(s))
        .any(|existing| existing.display() == canonical)
    {
        persisted
            .exclusion
            .extra_website_patterns
            .push(canonical.clone());
    }
    state
        .config_service
        .update(persisted)
        .map_err(String::from)?;
    website_exclusion_list(state).await
}

#[tauri::command]
pub async fn website_exclusion_remove(
    state: State<'_, AppState>,
    pattern: String,
) -> Result<Vec<WebsiteExclusionEntry>, String> {
    let target =
        crate::services::website_exclusion::WebsitePattern::parse(&pattern).map(|p| p.display());
    let mut persisted = state.config_service.get();
    persisted.exclusion.extra_website_patterns.retain(|stored| {
        let Some(parsed) = crate::services::website_exclusion::WebsitePattern::parse(stored) else {
            return false; // drop unparseable entries while we're at it
        };
        match &target {
            Some(t) => parsed.display() != *t,
            None => true,
        }
    });
    state
        .config_service
        .update(persisted)
        .map_err(String::from)?;
    website_exclusion_list(state).await
}
