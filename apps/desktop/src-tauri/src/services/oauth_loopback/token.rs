//! Token endpoint POST — both the initial code-for-token exchange
//! and the refresh_token flow. Manual reqwest POST instead of
//! `oauth2::Client::exchange_code` because we need the full token
//! response (id_token, refresh_token, scope) which the typed
//! `BasicTokenResponse` doesn't surface.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{CorivoError, Result};

/// Result of a token endpoint POST. Field availability depends on
/// scope + flow:
///   - `id_token` populated only when `openid` scope is in play.
///   - `refresh_token` populated only when the IdP granted offline
///     access (Google: `access_type=offline` + first-time consent).
///   - `scope` is whatever the IdP *actually* granted, which may be
///     a subset of what was requested (Google granular permissions).
///   - `raw_response` carries the full JSON for providers that hide
///     fields outside the OAuth-standard envelope. Slack puts its
///     user token at `authed_user.access_token` — the caller pulls
///     that out via `raw_response` after the standard parse.
#[derive(Debug, Default, Clone)]
pub struct TokenSet {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub token_type: String,
    /// Absolute wall-clock expiry. Computed from `expires_in` at
    /// response time so downstream code doesn't have to remember
    /// "is this relative or absolute".
    pub expires_at: Option<DateTime<Utc>>,
    /// Granted scopes (the IdP returns these space-separated; we split).
    pub scope: Vec<String>,
    /// Full untouched token-endpoint response JSON. None only when the
    /// upstream body wasn't valid JSON (i.e. never, in practice — we'd
    /// have errored out parsing it).
    pub raw_response: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

fn parse_token_response(value: serde_json::Value) -> Result<TokenSet> {
    let raw: RawTokenResponse = serde_json::from_value(value.clone())
        .map_err(|e| CorivoError::Internal(format!("token response parse failed: {e}")))?;
    let expires_at = raw
        .expires_in
        .and_then(|s| chrono::Duration::try_seconds(s).map(|d| Utc::now() + d));
    let scope = raw
        .scope
        .map(|s| {
            s.split_whitespace()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(TokenSet {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        id_token: raw.id_token,
        token_type: raw.token_type.unwrap_or_else(|| "Bearer".to_string()),
        expires_at,
        scope,
        raw_response: Some(value),
    })
}

fn build_http_client() -> Result<reqwest::Client> {
    reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| CorivoError::Internal(format!("http client: {e}")))
}

/// Per Google's docs (and OAuth 2.0 in general):
///   POST {token_url}
///   Content-Type: application/x-www-form-urlencoded
///   code=<code>&client_id=<id>[&client_secret=<secret>]
///   &redirect_uri=<uri>&grant_type=authorization_code
///   &code_verifier=<pkce_verifier>
pub(super) async fn exchange_code(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_uri: &str,
    code: &str,
    pkce_verifier: &str,
) -> Result<TokenSet> {
    let http = build_http_client()?;

    let mut form: Vec<(&str, String)> = vec![
        ("code", code.to_string()),
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("code_verifier", pkce_verifier.to_string()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.to_string()));
    }

    let response = http
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| CorivoError::Internal(format!("token exchange network error: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CorivoError::Internal(format!(
            "token exchange failed ({status}): {body}"
        )));
    }

    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| CorivoError::Internal(format!("token response json failed: {e}")))?;
    parse_token_response(value)
}

/// Refresh flow:
///   POST {token_url}
///   client_id=<id>[&client_secret=<secret>]
///   &grant_type=refresh_token&refresh_token=<rt>
///
/// Note: Google typically does NOT rotate refresh_token on refresh
/// (the response usually omits it). If it ever does, callers should
/// prefer the rotated value over the original.
pub(super) async fn refresh_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenSet> {
    let http = build_http_client()?;

    let mut form: Vec<(&str, String)> = vec![
        ("client_id", client_id.to_string()),
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.to_string()));
    }

    let response = http
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| CorivoError::Internal(format!("token refresh network error: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CorivoError::Internal(format!(
            "token refresh failed ({status}): {body}"
        )));
    }

    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| CorivoError::Internal(format!("token refresh response json failed: {e}")))?;
    parse_token_response(value)
}
