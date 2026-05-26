//! "Sign in with ChatGPT" — OAuth 2.0 PKCE loopback flow that lets
//! ChatGPT Plus/Pro/Business/Enterprise/Edu subscribers authorize
//! Corivo's executive-agent sidecar to call OpenAI Codex models
//! against their ChatGPT subscription quota.
//!
//! ## Wire contract (mirrors OpenAI Codex CLI)
//!
//! * Auth endpoint:    `https://auth.openai.com/oauth/authorize`
//! * Token endpoint:   `https://auth.openai.com/oauth/token`
//! * Public client_id: `app_EMoamEEZ73f0CkXaXp7hrann` (shared with the
//!   official Codex CLI — there is no third-party OAuth registration
//!   program; the PKCE flow has no secret so reuse is mechanically
//!   safe. Several other apps in the wild reuse it the same way —
//!   Zed, opencode, etc.)
//! * Redirect URI:     `http://localhost:1455/auth/callback` (fixed
//!   port; matches what Codex CLI registers. Fallback to `1457` if
//!   1455 is in use locally. Path is `/auth/callback`, not the
//!   default `/callback`.)
//! * Scopes:           `openid profile email offline_access
//!   api.connectors.read api.connectors.invoke`
//! * Extra params:     `id_token_add_organizations=true` +
//!   `codex_cli_simplified_flow=true`
//! * Originator header sent on every model call: `corivo_desktop`
//!   (set sidecar-side; documented here for grep-ability).
//!
//! ## Token storage
//!
//! Persisted plaintext in `Config.exec_agent.chatgpt` — same trust
//! model as `byok_key` (no Keychain). The id_token JWT's
//! `https://api.openai.com/auth.chatgpt_account_id` claim becomes
//! `account_id` (mandatory — the `ChatGPT-Account-Id` header on
//! every request is keyed off it). `plan_type` + `email` come from
//! the same claim namespace and surface in the Settings status pane.
//!
//! ## Refresh
//!
//! `refresh_session` trades the stored `refresh_token` for a fresh
//! `(access_token, refresh_token, id_token)` triple at the same
//! token endpoint, then re-derives `account_id` / `plan_type` /
//! `email` from the new id_token. Called by
//! `services::exec_agent::runtime` ~5 minutes before
//! `expires_at` so the sidecar never sees an expired bearer.
//!
//! ## Legal note
//!
//! OpenAI publishes no third-party Sign-in-with-ChatGPT program. The
//! flow works because (a) PKCE has no client secret and (b)
//! chatgpt.com/backend-api/codex accepts our originator string. They
//! may close either gap without notice — surface this to the user
//! as a clearly-labeled "experimental" path and keep BYOK as the
//! supported fallback.

use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::domain::config::ChatgptAuthConfig;
use crate::error::{CorivoError, Result};
use crate::services::oauth_loopback::{LoopbackFlow, TokenSet};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const PRIMARY_PORT: u16 = 1455;
const FALLBACK_PORT: u16 = 1457;
const CALLBACK_PATH: &str = "/auth/callback";
const LOOPBACK_HOST: &str = "localhost";
/// 5-minute end-to-end budget. ChatGPT's consent screen is fast when
/// the user is already logged in; the timeout exists so a closed
/// browser tab doesn't leak the listener forever.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "api.connectors.read",
    "api.connectors.invoke",
];

/// Outcome of a successful login or refresh. Mirrors the persisted
/// shape so the caller can drop it straight into Config.
#[derive(Debug)]
pub struct ChatgptSession {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: String,
    pub plan_type: Option<String>,
    pub email: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl ChatgptSession {
    /// Convert to the persisted Config shape (RFC3339 timestamps,
    /// `None` for missing fields).
    pub fn into_config(self) -> ChatgptAuthConfig {
        ChatgptAuthConfig {
            access_token: Some(self.access_token),
            refresh_token: self.refresh_token,
            id_token: self.id_token,
            account_id: Some(self.account_id),
            plan_type: self.plan_type,
            email: self.email,
            expires_at: self.expires_at.map(|d| d.to_rfc3339()),
        }
    }
}

/// Run the full PKCE loopback flow and return a `ChatgptSession`. The
/// caller wires the returned struct into `Config.exec_agent.chatgpt`
/// and writes via `ConfigService`.
///
/// Tries port 1455 first; falls back to 1457 on EADDRINUSE so a
/// concurrent ChatGPT desktop login on the host machine doesn't
/// block us. Both ports must be registered on OpenAI's side for the
/// shared client_id — Codex CLI documents both as accepted.
pub async fn login(app: &AppHandle) -> Result<ChatgptSession> {
    let tokens = match try_login_at(app, PRIMARY_PORT).await {
        Ok(t) => t,
        Err(err) if is_addr_in_use(&err) => {
            tracing::warn!(
                target: "chatgpt_auth",
                error = %err,
                "chatgpt_auth.login.primary_port_in_use_retry_fallback"
            );
            try_login_at(app, FALLBACK_PORT).await?
        }
        Err(err) => return Err(err),
    };
    derive_session(tokens)
}

/// Trade an existing refresh_token for a fresh credential triple.
/// Returns the new session — the new id_token's claims are
/// re-parsed so a plan upgrade / email change is reflected in
/// Settings without forcing a full re-login.
pub async fn refresh_session(refresh_token: &str) -> Result<ChatgptSession> {
    let flow = build_flow(PRIMARY_PORT);
    let tokens = flow.refresh(refresh_token).await?;
    derive_session(tokens)
}

async fn try_login_at(app: &AppHandle, port: u16) -> Result<TokenSet> {
    let flow = build_flow(port);
    let app_for_browser = app.clone();
    let open_browser = move |url: String| {
        app_for_browser
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| CorivoError::Internal(format!("Could not open browser: {e}")))
    };
    let started_at = std::time::Instant::now();
    tracing::info!(
        target: "chatgpt_auth",
        port,
        "chatgpt_auth.login.flow_start"
    );
    let tokens = tokio::time::timeout(LOGIN_TIMEOUT, flow.run(open_browser))
        .await
        .map_err(|_| {
            CorivoError::Internal(
                "ChatGPT sign-in timed out (5 min) — close the browser tab and retry".to_string(),
            )
        })??;
    tracing::info!(
        target: "chatgpt_auth",
        port,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "chatgpt_auth.login.flow_done"
    );
    Ok(tokens)
}

fn build_flow(port: u16) -> LoopbackFlow {
    LoopbackFlow {
        client_id: CLIENT_ID.to_string(),
        client_secret: None,
        auth_url: AUTH_URL.to_string(),
        token_url: TOKEN_URL.to_string(),
        scopes: SCOPES.iter().map(|s| (*s).to_string()).collect(),
        extra_auth_params: vec![
            ("id_token_add_organizations".to_string(), "true".to_string()),
            (
                "codex_cli_simplified_flow".to_string(),
                "true".to_string(),
            ),
        ],
        // Redirect after consent. OpenAI's auth flow already shows a
        // success screen before redirect, so we send the user back to
        // their ChatGPT session rather than rendering an extra page.
        success_url: "https://chatgpt.com/".to_string(),
        error_url_prefix: "https://chatgpt.com/".to_string(),
        fixed_port: Some(port),
        callback_path: Some(CALLBACK_PATH.to_string()),
        loopback_host: Some(LOOPBACK_HOST.to_string()),
    }
}

fn is_addr_in_use(err: &CorivoError) -> bool {
    matches!(err, CorivoError::Internal(msg) if msg.contains("port") && msg.contains("in use"))
        || matches!(err, CorivoError::Internal(msg) if msg.contains("address already in use"))
}

/// Pull the ChatGPT-specific claims out of a TokenSet's id_token JWT
/// and assemble a `ChatgptSession`. Errors out if the id_token is
/// missing the `chatgpt_account_id` claim — without that header the
/// chatgpt.com endpoint returns 401 on every call, so there is no
/// graceful fallback.
fn derive_session(tokens: TokenSet) -> Result<ChatgptSession> {
    let access_token = tokens.access_token.clone().ok_or_else(|| {
        CorivoError::Internal(
            "ChatGPT token endpoint returned no access_token".to_string(),
        )
    })?;
    let id_token = tokens.id_token.clone();
    let claims = id_token
        .as_deref()
        .map(decode_jwt_claims)
        .transpose()?
        .unwrap_or_default();
    let account_id = claims
        .chatgpt_account_id
        .clone()
        .ok_or_else(|| {
            CorivoError::Internal(
                "ChatGPT id_token missing chatgpt_account_id claim — \
                 a free-tier or non-eligible ChatGPT account cannot use this flow"
                    .to_string(),
            )
        })?;
    Ok(ChatgptSession {
        access_token,
        refresh_token: tokens.refresh_token,
        id_token,
        account_id,
        plan_type: claims.chatgpt_plan_type,
        email: claims.email,
        expires_at: tokens.expires_at,
    })
}

/// Subset of the OIDC id_token JWT claims we care about. Codex CLI
/// nests the ChatGPT-account fields under the
/// `https://api.openai.com/auth` namespace, mirroring Auth0's
/// custom-claim convention.
#[derive(Debug, Default, Deserialize)]
struct IdTokenClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    openai_auth: OpenAiAuthClaims,
    #[serde(skip)]
    chatgpt_account_id: Option<String>,
    #[serde(skip)]
    chatgpt_plan_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiAuthClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
}

fn decode_jwt_claims(jwt: &str) -> Result<IdTokenClaims> {
    // JWT = base64url(header).base64url(payload).signature — we only
    // need the middle segment and don't verify the signature (the
    // token came over TLS from auth.openai.com against our own PKCE
    // verifier; treating it as authoritative is the same posture
    // Codex CLI takes).
    let segments: Vec<&str> = jwt.split('.').collect();
    if segments.len() < 2 {
        return Err(CorivoError::Internal(
            "ChatGPT id_token has fewer than 2 segments — malformed JWT".to_string(),
        ));
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segments[1])
        .map_err(|e| CorivoError::Internal(format!("id_token base64 decode: {e}")))?;
    let mut claims: IdTokenClaims = serde_json::from_slice(&payload)
        .map_err(|e| CorivoError::Internal(format!("id_token claims parse: {e}")))?;
    claims.chatgpt_account_id = claims.openai_auth.chatgpt_account_id.clone();
    claims.chatgpt_plan_type = claims.openai_auth.chatgpt_plan_type.clone();
    tracing::debug!(
        target: "chatgpt_auth",
        has_account_id = claims.chatgpt_account_id.is_some(),
        plan_type = claims.chatgpt_plan_type.as_deref().unwrap_or(""),
        has_user_id = claims.openai_auth.chatgpt_user_id.is_some(),
        has_email = claims.email.is_some(),
        "chatgpt_auth.decode_jwt_claims.done"
    );
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(payload: &serde_json::Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(payload).unwrap());
        format!("{header}.{payload_b64}.sig")
    }

    #[test]
    fn jwt_decode_extracts_namespaced_chatgpt_account_id() {
        let payload = serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-abc-123",
                "chatgpt_plan_type": "pro",
                "chatgpt_user_id": "user-xyz",
            },
        });
        let claims = decode_jwt_claims(&make_jwt(&payload)).unwrap();
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("acct-abc-123"));
        assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("pro"));
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn jwt_decode_handles_missing_namespace_block() {
        let payload = serde_json::json!({ "email": "u@example.com" });
        let claims = decode_jwt_claims(&make_jwt(&payload)).unwrap();
        assert!(claims.chatgpt_account_id.is_none());
        assert!(claims.chatgpt_plan_type.is_none());
        assert_eq!(claims.email.as_deref(), Some("u@example.com"));
    }

    #[test]
    fn jwt_decode_rejects_one_segment_jwt() {
        let err = decode_jwt_claims("not-a-jwt").unwrap_err();
        assert!(matches!(err, CorivoError::Internal(msg) if msg.contains("fewer than 2 segments")));
    }

    #[test]
    fn derive_session_rejects_missing_account_id() {
        let mut tokens = TokenSet::default();
        tokens.access_token = Some("at".to_string());
        tokens.id_token = Some(make_jwt(&serde_json::json!({ "email": "u@e.com" })));
        let err = derive_session(tokens).unwrap_err();
        assert!(matches!(err, CorivoError::Internal(msg) if msg.contains("chatgpt_account_id")));
    }

    #[test]
    fn derive_session_assembles_session_from_jwt() {
        let mut tokens = TokenSet::default();
        tokens.access_token = Some("at".to_string());
        tokens.refresh_token = Some("rt".to_string());
        tokens.id_token = Some(make_jwt(&serde_json::json!({
            "email": "u@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1",
                "chatgpt_plan_type": "plus",
            },
        })));
        let s = derive_session(tokens).unwrap();
        assert_eq!(s.access_token, "at");
        assert_eq!(s.refresh_token.as_deref(), Some("rt"));
        assert_eq!(s.account_id, "acct-1");
        assert_eq!(s.plan_type.as_deref(), Some("plus"));
        assert_eq!(s.email.as_deref(), Some("u@example.com"));
    }
}
