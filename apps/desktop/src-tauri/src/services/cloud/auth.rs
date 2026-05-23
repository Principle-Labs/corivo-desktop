//! Authentication capability.
//!
//! Closed-source impl: `cloud::corivo::auth` — Google OAuth loopback
//! + email OTP against the closed Corivo backend, with a refresh-token rotation
//! stored in `config.json`.
//!
//! Open-source impl: [`super::noop::NoopAuthService`] — every method
//! returns `CorivoError::FeatureUnavailable { feature: "auth" }`. The
//! `/login` route is hidden from the router and the user-avatar slot
//! becomes a settings-only "本地用户" placeholder.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    /// True when a refresh token is on disk. Says nothing about
    /// whether the access token is currently valid server-side —
    /// `auth_refresh` asks the cloud auth service for the source of
    /// truth.
    pub logged_in: bool,
    /// Operator-supplied label for the current beta account (e.g.
    /// "alice (early-access)"). Only present when creds have been
    /// refreshed at least once this session.
    pub label: Option<String>,
    /// ISO-8601 expiry of the cached access token. Re-issued on
    /// every successful refresh, so this surface drifts forward as
    /// the user keeps using the app.
    pub access_expires_at: Option<String>,
    /// ISO-8601 expiry of the cached refresh token. Sliding — every
    /// successful refresh resets it to `now + 30d`. When `now`
    /// crosses this, the user is forced back through Google sign-in.
    pub refresh_expires_at: Option<String>,
    /// The API base the desktop is talking to. Useful diagnostic in
    /// the settings UI when the dev/prod toggle isn't obvious.
    pub api_base: String,
    /// Verified Google email of the signed-in user. Mirrors
    /// `accounts.google_email` server-side.
    pub email: Option<String>,
    /// Display name from the Google ID token.
    pub name: Option<String>,
    /// Profile picture URL from the Google ID token.
    pub avatar_url: Option<String>,
}

/// Public Google profile bits surfaced in the UI (sidebar avatar,
/// settings page, ...). Always populated for a logged-in cloud
/// session; `name` and `avatar_url` are nullable upstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub email: String,
    pub name: Option<String>,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: Option<String>,
}

/// Outcome of requesting an email login code. We treat the rate-limited
/// response as a regular outcome (not an error) so the UI can render
/// the cooldown inline instead of through the generic failure pane.
/// Anything else (5xx, network, malformed body) bubbles up as
/// `CorivoError` and becomes a `TauriError` at the command boundary.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestEmailCodeOutcome {
    Sent {
        #[serde(rename = "expiresInSeconds")]
        expires_in_seconds: u32,
    },
    RateLimited {
        #[serde(rename = "retryAfterSeconds")]
        retry_after_seconds: u32,
        message: String,
    },
}

/// Subset of email-code verification failures the UI can recover from
/// inline (user retypes the code, or hits "重发"). Anything outside
/// this enum (network / 5xx) flows through `TauriError`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum EmailRejectCode {
    InvalidCode,
    CodeExpired,
    TooManyAttempts,
}

/// Internal result of `verify_email_code`; the command layer wraps the
/// `Ok` branch with an `AuthStatus` snapshot for the frontend.
#[derive(Debug, Clone)]
pub enum EmailVerifyResult {
    Ok,
    Rejected {
        code: EmailRejectCode,
        message: String,
    },
}

/// Cloud sign-in + session lifecycle.
///
/// Methods follow the same shape the existing `commands::auth::*`
/// handlers expect: the trait owns the cloud round-trip (token
/// exchange, refresh-token rotation, account probe); command-layer
/// concerns (Sentry user sync, side-effect catalog refresh) stay in
/// `commands::auth`.
#[async_trait]
pub trait AuthService: Send + Sync {
    /// True for the closed Corivo build, false for the open-source
    /// noop. Surfaces through `Capabilities.auth` so the frontend can
    /// hide the login route + avatar widgets cleanly.
    fn is_available(&self) -> bool {
        false
    }

    /// True when a refresh token is on disk. Says nothing about whether
    /// the access token is currently valid server-side — that's what
    /// `refresh` is for.
    async fn has_session(&self) -> Result<bool>;

    /// UI-facing snapshot: account email + token expiries. Does NOT
    /// hit the network; use `refresh` for a server round-trip.
    async fn status(&self) -> Result<AuthStatus>;

    /// Run the Google OAuth loopback flow + trade the resulting
    /// `id_token` for a cloud session. Returns the freshly-loaded
    /// `AuthStatus`. Idempotent: re-running while already signed in
    /// replaces the token (handy when switching Google accounts).
    async fn login_google(&self) -> Result<AuthStatus>;

    /// Trigger sending a 6-digit OTP. The `RateLimited` outcome is
    /// part of the success channel (not an error) so the UI can render
    /// an accurate cooldown countdown on the "重发" button.
    async fn request_email_code(&self, email: String) -> Result<RequestEmailCodeOutcome>;

    /// Verify a 6-digit OTP. `EmailVerifyResult::Rejected` covers the
    /// user-recoverable 401 family (invalid code / expired / too many
    /// attempts); other failures bubble as `Err`.
    async fn login_email(&self, email: String, code: String) -> Result<EmailVerifyResult>;

    /// Best-effort: tell the server to invalidate the current session,
    /// then wipe local state regardless of the network outcome.
    async fn logout(&self) -> Result<()>;

    /// Force a cloud account round-trip (with pre-emptive token refresh
    /// if needed) and return the updated `AuthStatus`. Used at app
    /// boot and behind the "重新登录" button.
    async fn refresh(&self) -> Result<AuthStatus>;
}
