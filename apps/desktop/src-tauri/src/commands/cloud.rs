//! Tauri command surface for the cloud-capability probe.
//!
//! Frontend fetches this exactly once at app boot and caches the
//! result in a `useCapabilities()` React Query hook. Every cloud-
//! coupled UI surface (the `/login` route, `BillingDialog`,
//! Settings → Integrations, model directory picker, updater UI,
//! Sentry init) reads the matching boolean and either renders or
//! short-circuits.
//!
//! In the open-source build every field is `false`. In the closed
//! Corivo build every field is `true` (or `false` if a particular
//! capability failed to initialise — e.g. `models_directory` is `false`
//! when `app_data_dir` was unavailable at boot, since the catalog
//! couldn't be constructed).
//!
//! Cheap: just reads the `is_available()` / `is_managed()` booleans
//! that the trait objects advertise about themselves — no network,
//! no disk, no allocation.

use tauri::State;

use crate::commands::config::AppState;
use crate::services::cloud::Capabilities;

#[tauri::command]
pub async fn get_capabilities(state: State<'_, AppState>) -> Result<Capabilities, ()> {
    Ok(state.cloud.capabilities())
}
