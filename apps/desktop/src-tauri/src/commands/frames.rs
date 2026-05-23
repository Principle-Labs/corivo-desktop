//! `/timeline` and `/ask` (Phase 3) read frames through these commands.
//!
//! Phase 1 surface:
//!   - `frames_list`  — paginated, time-desc, optional app/url filter.
//!   - `frame_detail` — single frame by id (no screenshot).
//!
//! Phase 3 will add `frames_search` (FTS) and `get_current_screen` (live
//! capture); not in this file yet.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tauri::State;

use crate::{
    commands::config::AppState, db::repos::frames::ListFramesOptions, domain::frame::Frame,
};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ListFramesArgs {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub app_bundle_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[tauri::command]
pub async fn frames_list(
    state: State<'_, AppState>,
    args: ListFramesArgs,
) -> Result<Vec<Frame>, String> {
    let repo = state
        .frames_repo
        .as_ref()
        .ok_or_else(|| "frames repo not yet initialized".to_string())?
        .clone();
    let opts = ListFramesOptions {
        from: args.from,
        to: args.to,
        app_bundle_id: args.app_bundle_id,
        limit: args.limit.unwrap_or(50).min(500),
        offset: args.offset.unwrap_or(0),
    };
    repo.list(opts).await.map_err(String::from)
}

#[tauri::command]
pub async fn frame_detail(state: State<'_, AppState>, id: String) -> Result<Option<Frame>, String> {
    let repo = state
        .frames_repo
        .as_ref()
        .ok_or_else(|| "frames repo not yet initialized".to_string())?
        .clone();
    repo.by_id(&id).await.map_err(String::from)
}
