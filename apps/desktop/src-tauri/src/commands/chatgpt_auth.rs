//! Tauri command surface for "Sign in with ChatGPT".
//!
//! Three commands the Settings UI calls:
//!   * `chatgpt_auth_login`  — run the PKCE loopback flow, persist
//!     creds into `Config.exec_agent.chatgpt`, return the status the
//!     UI renders.
//!   * `chatgpt_auth_logout` — wipe the persisted creds. The exec
//!     agent mode itself is not flipped back to `Byok` — that's a
//!     deliberate user-driven action in the Settings radio group.
//!   * `chatgpt_auth_status` — read the current persisted creds and
//!     surface a derived status object (no token material).

use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::domain::config::ChatgptAuthConfig;
use crate::domain::ipc_error::TauriError;
use crate::services::chatgpt_auth;

/// Stripped-down view of `ChatgptAuthConfig` for the frontend. Never
/// carries token material — only the human-readable bits the
/// Settings status pane shows.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ChatgptAuthStatus {
    pub signed_in: bool,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    /// RFC3339; UI computes the relative "expires in N min" for free.
    pub expires_at: Option<String>,
    pub account_id: Option<String>,
}

impl From<&ChatgptAuthConfig> for ChatgptAuthStatus {
    fn from(cfg: &ChatgptAuthConfig) -> Self {
        Self {
            signed_in: cfg.is_signed_in(),
            email: cfg.email.clone(),
            plan_type: cfg.plan_type.clone(),
            expires_at: cfg.expires_at.clone(),
            account_id: cfg.account_id.clone(),
        }
    }
}

#[tauri::command]
pub async fn chatgpt_auth_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChatgptAuthStatus, TauriError> {
    let session = chatgpt_auth::login(&app).await.map_err(|err| {
        tracing::error!(
            target: "chatgpt_auth",
            error = %err,
            "chatgpt_auth.login.failed"
        );
        err
    })?;
    let new_chatgpt = session.into_config();
    let mut cfg = state.config_service.get();
    cfg.exec_agent.chatgpt = new_chatgpt.clone();
    state
        .config_service
        .update(cfg)
        .map_err(|e| TauriError::from(crate::error::CorivoError::Internal(e.to_string())))?;
    tracing::info!(
        target: "chatgpt_auth",
        plan_type = new_chatgpt.plan_type.as_deref().unwrap_or(""),
        has_email = new_chatgpt.email.is_some(),
        "chatgpt_auth.login.persisted"
    );
    Ok(ChatgptAuthStatus::from(&new_chatgpt))
}

#[tauri::command]
pub async fn chatgpt_auth_logout(
    state: State<'_, AppState>,
) -> Result<ChatgptAuthStatus, TauriError> {
    let mut cfg = state.config_service.get();
    cfg.exec_agent.chatgpt = ChatgptAuthConfig::default();
    state
        .config_service
        .update(cfg)
        .map_err(|e| TauriError::from(crate::error::CorivoError::Internal(e.to_string())))?;
    tracing::info!(target: "chatgpt_auth", "chatgpt_auth.logout.done");
    Ok(ChatgptAuthStatus::from(&ChatgptAuthConfig::default()))
}

#[tauri::command]
pub async fn chatgpt_auth_status(
    state: State<'_, AppState>,
) -> Result<ChatgptAuthStatus, TauriError> {
    let cfg = state.config_service.get();
    Ok(ChatgptAuthStatus::from(&cfg.exec_agent.chatgpt))
}
