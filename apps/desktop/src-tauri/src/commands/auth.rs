//! Tauri command surface for the cloud sign-in flow.
//!
//! Frontend boot calls `auth_status` to decide whether to route to
//! `/login` or the main shell. Login page calls `auth_login_google` /
//! `auth_login_email`. Settings page calls `auth_logout`. The runner
//! (executive agent) reads creds via the `AgentCredsProvider` trait.
//!
//! Every method here is a thin Tauri-boundary wrapper around
//! `state.cloud.auth.*`. In the closed Corivo build the trait points at
//! `cloud::corivo::auth::CorivoAuthService` (loopback + backend-owned
//! PKCE exchange); in the open-source build it
//! points at `cloud::noop::NoopAuthService` and every method returns
//! `CorivoError::FeatureUnavailable { feature: "auth" }`. The frontend
//! hides the entire `/login` route in that case via the capability
//! probe.

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::commands::config::AppState;
use crate::domain::ipc_error::TauriError;
use crate::error::CorivoError;
use crate::services::cloud::auth::{
    AuthStatus, EmailRejectCode, EmailVerifyResult, RequestEmailCodeOutcome,
};

/// Mirror the current login state into the global Sentry scope so every
/// captured event is attributed to the right Gmail account. Idempotent:
/// every auth_* command runs this on its way out, so login/logout/refresh
/// + boot-time `auth_status` all converge on the right value.
///
/// We use the Gmail address as both `id` and `email`. `id` makes Sentry's
/// "users affected" count work (it dedupes by id); `email` lights up the
/// account row in the issue detail. In the open-source build Sentry is
/// never initialised, so these calls are cheap no-ops.
fn sync_sentry_user(status: &AuthStatus) {
    sentry::configure_scope(|scope| {
        if status.logged_in {
            scope.set_user(Some(sentry::User {
                id: status.email.clone(),
                email: status.email.clone(),
                username: status.name.clone(),
                ..Default::default()
            }));
        } else {
            scope.set_user(None);
        }
    });
}

/// Outcome of `auth_login_email`. The `Ok` branch carries the same
/// `AuthStatus` snapshot `auth_login_google` returns, so the UI can
/// hand it straight to `applyAuthStatus`. The `Rejected` branch lets
/// the UI render an inline retry hint (wrong code / expired / out of
/// attempts) without going through the failure pane.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EmailLoginOutcome {
    Ok {
        auth: AuthStatus,
    },
    Rejected {
        code: EmailRejectCode,
        message: String,
    },
}

/// 区分"用户可恢复的失败" vs "真错误"：前者只打 info breadcrumb，
/// 后者走 tracing::error → Sentry event。共享给 login_google / login_email
/// 两个入口。
fn log_login_error(target_event: &'static str, err: &CorivoError) {
    let user_aborted_or_denied = matches!(err, CorivoError::AuthDenied { .. })
        || matches!(err, CorivoError::Internal(msg) if msg.contains("timed out"));
    if user_aborted_or_denied {
        tracing::info!(
            target: "auth",
            error = %err,
            event = target_event,
            "auth.login.user_aborted_or_denied"
        );
    } else {
        tracing::error!(
            target: "auth",
            error = %err,
            event = target_event,
            "auth.login.failed"
        );
    }
}

/// Run the Google OAuth loopback flow, exchange the resulting ID token
/// for a cloud session, and stash the creds. Idempotent: calling it
/// while already logged in re-runs the whole thing and replaces the
/// token (handy if the user wants to switch Google accounts).
///
/// In the open-source build this returns
/// `TauriError::FeatureUnavailable { feature: "auth" }` — the `/login`
/// route should already be hidden by the frontend's capability check,
/// but this is the last-line guarantee.
#[tauri::command]
pub async fn auth_login_google(state: State<'_, AppState>) -> Result<AuthStatus, TauriError> {
    let status = state.cloud.auth.login_google().await.map_err(|err| {
        log_login_error("login_google", &err);
        TauriError::from(err)
    })?;
    sync_sentry_user(&status);
    Ok(status)
}

/// Trigger sending a 6-digit OTP code to `email`. Returns the
/// outcome verbatim (Sent / RateLimited) — the UI uses the
/// `retryAfterSeconds` on RateLimited to render an accurate cooldown.
#[tauri::command]
pub async fn auth_request_email_code(
    state: State<'_, AppState>,
    email: String,
) -> Result<RequestEmailCodeOutcome, TauriError> {
    state
        .cloud
        .auth
        .request_email_code(email)
        .await
        .map_err(|err| {
            tracing::error!(
                target: "auth",
                error = %err,
                "auth.email.request_code.failed"
            );
            err.into()
        })
}

/// Verify the OTP code and, on success, mint a cloud session
/// (same shape Google login produces). Recoverable 401s come back as
/// `EmailLoginOutcome::Rejected`; 5xx / network errors bubble as
/// `TauriError`.
#[tauri::command]
pub async fn auth_login_email(
    state: State<'_, AppState>,
    email: String,
    code: String,
) -> Result<EmailLoginOutcome, TauriError> {
    let verify = state
        .cloud
        .auth
        .login_email(email, code)
        .await
        .map_err(|err| {
            log_login_error("login_email", &err);
            TauriError::from(err)
        })?;

    match verify {
        EmailVerifyResult::Rejected { code, message } => {
            Ok(EmailLoginOutcome::Rejected { code, message })
        }
        EmailVerifyResult::Ok => {
            // 与 Google 登录对齐：trait impl 已经在内部派发了 model
            // catalog 的刷新，这里只剩 Sentry 同步 + 把最新 status
            // 透传给前端。
            let status = state.cloud.auth.status().await?;
            sync_sentry_user(&status);
            Ok(EmailLoginOutcome::Ok { auth: status })
        }
    }
}

#[tauri::command]
pub async fn auth_logout(state: State<'_, AppState>) -> Result<(), TauriError> {
    state.cloud.auth.logout().await?;
    sync_sentry_user(&AuthStatus {
        logged_in: false,
        label: None,
        access_expires_at: None,
        refresh_expires_at: None,
        api_base: String::new(),
        email: None,
        name: None,
        avatar_url: None,
    });
    Ok(())
}

#[tauri::command]
pub async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, TauriError> {
    let status = state.cloud.auth.status().await?;
    sync_sentry_user(&status);
    Ok(status)
}

#[tauri::command]
pub async fn auth_refresh(state: State<'_, AppState>) -> Result<AuthStatus, TauriError> {
    // Trait impl pre-emptively rotates the access token if needed and
    // asks the cloud service for fresh creds. Surface the underlying
    // error so the UI can show "session expired" instead of a generic
    // spinner.
    let status = state.cloud.auth.refresh().await?;
    sync_sentry_user(&status);
    Ok(status)
}
