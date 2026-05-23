use tauri::State;

use crate::commands::config::AppState;

/// Bumped 2 → 3 with the redesign in
/// docs/design/onboarding-redesign-v2.html. The 3 steps are:
///
///   1. Permission  — single-column layout, per-row "授权" buttons.
///                    Auto-advances 800 ms after both grants land.
///   2. Demo        — static showcase of the product (mock PRD +
///                    Quick Ask + Apple Notes feedback card). User
///                    clicks "继续" to move on. No real LLM call.
///   3. Shortcut    — keyboard hero with the ⌥⌥ summon affordance.
///                    The "进入 Corivo" button is what marks
///                    onboarding complete (no longer driven by the
///                    `onboarding:first-quick-ask` event).
///
/// Existing users on `v2` (warmup + try-it) get re-onboarded once
/// on next launch. Pre-release, the re-prompt is acceptable.
pub const CURRENT_ONBOARDING_VERSION: u32 = 3;

#[derive(Debug, serde::Serialize)]
pub struct OnboardingState {
    pub completed: bool,
    pub version: u32,
    pub current_version: u32,
    pub needs_onboarding: bool,
    pub step: Option<String>,
}

#[tauri::command]
pub async fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let config = state.config_service.get();

    Ok(OnboardingState {
        completed: config.app.onboarding_completed,
        version: config.app.onboarding_version,
        current_version: CURRENT_ONBOARDING_VERSION,
        needs_onboarding: !config.app.onboarding_completed
            || config.app.onboarding_version < CURRENT_ONBOARDING_VERSION,
        step: config.app.onboarding_step.clone(),
    })
}

#[tauri::command]
pub async fn save_onboarding_step(state: State<'_, AppState>, step: String) -> Result<(), String> {
    let mut config = state.config_service.get();
    config.app.onboarding_step = Some(step);
    state.config_service.update(config).map_err(String::from)
}

#[tauri::command]
pub async fn mark_onboarding_completed(state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config_service.get();
    config.app.onboarding_completed = true;
    config.app.onboarding_version = CURRENT_ONBOARDING_VERSION;
    config.app.onboarding_step = None;
    // Demo step already showed the user what Quick Ask looks like
    // (suggestion chips, input). Flipping this here means their first
    // real ⌥⌥ summon goes straight to the empty-state instead of
    // re-rendering the onboarding chips they just saw.
    config.app.first_quick_ask_done = true;
    state.config_service.update(config).map_err(String::from)
}

#[tauri::command]
pub async fn reset_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config_service.get();
    config.app.onboarding_completed = false;
    config.app.onboarding_version = 0;
    config.app.onboarding_step = None;
    // Reset means "let me see the new-user experience again", which
    // includes the first-quick-ask suggestion chips.
    config.app.first_quick_ask_done = false;
    state.config_service.update(config).map_err(String::from)
}

/// Flip `Config.app.first_quick_ask_done` to `true`. Called by the
/// Quick Ask window once the user fires their very first turn during
/// the Try-It step. After this fires, Quick Ask stops showing the 3
/// suggestion chips and the onboarding window dismisses itself via
/// the `onboarding:first-quick-ask` Tauri event.
#[tauri::command]
pub async fn mark_first_quick_ask_done(state: State<'_, AppState>) -> Result<(), String> {
    let mut config = state.config_service.get();
    if config.app.first_quick_ask_done {
        // Already flipped — idempotent. Avoids double-writes when
        // the user fires several turns in quick succession.
        return Ok(());
    }
    config.app.first_quick_ask_done = true;
    state.config_service.update(config).map_err(String::from)
}
