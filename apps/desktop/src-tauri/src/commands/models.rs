//! Tauri commands surfacing the model directory to the frontend.
//!
//! Three entry points:
//!
//! * `models_get_available` — sync read of the on-disk cache. Returns
//!   the full directory (host + api_key + default_alias + managed[])
//!   so the picker can render without an extra round-trip. Always
//!   succeeds for the closed Corivo build (seeded demo set when the
//!   cache is empty). In the open-source build this returns
//!   `FeatureUnavailable` — the OSS Settings picker reads the user's
//!   own API key from `Config.exec_agent.anthropic_api_key` instead
//!   of asking for a managed directory.
//! * `models_refresh` — async fetch from `${gateway}/v1/me/models`
//!   followed by write-through to the cache.
//! * `models_set_active_model` — switch the user's active alias.
//!
//! All three delegate to `state.cloud.models.*`.

use serde::Deserialize;
use tauri::State;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;
use crate::services::model_catalog::ModelDirectory;

#[tauri::command]
pub async fn models_get_available(
    state: State<'_, AppState>,
) -> Result<ModelDirectory, TauriError> {
    Ok(state.cloud.models.get_available().await?)
}

#[tauri::command]
pub async fn models_refresh(state: State<'_, AppState>) -> Result<ModelDirectory, TauriError> {
    Ok(state.cloud.models.refresh().await?)
}

#[derive(Debug, Deserialize)]
pub struct ModelsSetActiveArgs {
    /// Alias the user picked from the composer dropdown. Must match
    /// one of the entries currently in the cache; if it doesn't, the
    /// backend will 400 and the frontend should refresh.
    pub alias: String,
}

/// `POST /v1/me/active-model` from the desktop side. Blocks until the
/// backend confirms the sub2api group flip — frontend should show a
/// loading state until this resolves. On success the trait impl writes
/// the new `active_alias` into the local cache, so the returned
/// directory already reflects the new state.
#[tauri::command]
pub async fn models_set_active_model(
    args: ModelsSetActiveArgs,
    state: State<'_, AppState>,
) -> Result<ModelDirectory, TauriError> {
    Ok(state.cloud.models.set_active_alias(args.alias).await?)
}
