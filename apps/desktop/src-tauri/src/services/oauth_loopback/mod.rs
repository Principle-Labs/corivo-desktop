//! Generic loopback OAuth 2.0 flow (Authorization Code + PKCE).
//!
//! Main caller today:
//!   - `services::connector` — third-party service connectors (Gmail, …),
//!     scopes per-service, consumer cares about `access_token` +
//!     `refresh_token` + granted `scope` set.
//! Corivo Google sign-in is owned by the private cloud auth overlay and
//! its backend-owned OAuth exchange, so the desktop never holds Google
//! client secrets.
//!
//! End-to-end (mirrors RFC 8252 §7.3 "Loopback Interface Redirection"):
//!   1. Mint a PKCE verifier + an opaque CSRF `state` string.
//!   2. Bind a tokio `TcpListener` on `127.0.0.1:0` (kernel-assigned
//!      ephemeral port). Read the port back out so the `redirect_uri`
//!      we register with the IdP matches what's actually listening.
//!   3. Build the authorize URL (auth_url + client_id + scope list +
//!      `redirect_uri` + `state` + `code_challenge` + caller-supplied
//!      extra params) and ask the system browser to open it.
//!   4. Accept exactly one inbound HTTP GET on the loopback listener,
//!      parse `code` + `state`, reject mismatches.
//!   5. Exchange the code at the IdP's token endpoint, surface the full
//!      token response (id_token, access_token, refresh_token, scope, …)
//!      back to the caller.
//!
//! Cancellation: callers MUST wrap `run()` in a timeout — if the user
//! closes the browser tab without completing consent we'd otherwise sit
//! at `accept()` forever.

pub use self::token::TokenSet;

mod server;
mod token;

use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope, TokenUrl};
use tokio::net::TcpListener;

use crate::error::{CorivoError, Result};

/// One full loopback OAuth round-trip: bind → open browser → wait for
/// callback → token exchange. Cloneable / re-usable; nothing mutates
/// across runs, each call mints a fresh PKCE verifier + state.
pub struct LoopbackFlow {
    pub client_id: String,
    /// Optional. For Google "Desktop app" OAuth clients the secret is
    /// not actually treated as a secret (see env.rs::google_client_secret
    /// doc comment); for "Web application" clients it's required.
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    /// Extra `?k=v` params appended to the authorize URL — `access_type=offline`,
    /// `prompt=consent`, `include_granted_scopes=true` for Gmail;
    /// `user_scope=...` for Slack (comma-separated). Empty for the
    /// corivo session login.
    pub extra_auth_params: Vec<(String, String)>,
    /// Where the loopback HTTP server 302-redirects the browser on
    /// success. Branded landing page, lives in `apps/web`.
    pub success_url: String,
    /// Prefix for the 302 redirect on failure. The loopback server
    /// appends `?reason=<url-encoded>`. The `reason` carries the
    /// upstream `error_description` (or `error` if no description).
    pub error_url_prefix: String,
    /// When `Some(port)`, bind the loopback listener to that exact
    /// port instead of asking the kernel for an ephemeral one.
    ///
    /// Why this exists: Google "Desktop app" OAuth clients accept any
    /// loopback port (RFC 8252 §7.3, explicitly documented by Google),
    /// so we default to ephemeral (`None`) and the kernel picks. Slack
    /// matches `redirect_uri` exactly against the URL registered in
    /// "OAuth & Permissions → Redirect URLs" — random ports fail. For
    /// Slack we pin a known port (currently 8989) and ask the user to
    /// register `http://127.0.0.1:8989/callback` once in app config.
    ///
    /// Trade-off: if the port is already in use (another OAuth flow,
    /// another Corivo install) `bind` returns EADDRINUSE and the user
    /// sees a clear error. We don't try to fall back to ephemeral —
    /// that would silently break the redirect URI match.
    pub fixed_port: Option<u16>,
}

impl LoopbackFlow {
    /// Run one full loopback round-trip and return the token response.
    ///
    /// `open_browser` is injected so test / dev paths can stub it;
    /// production passes a closure that dispatches via
    /// `tauri-plugin-opener`.
    pub async fn run<F>(&self, open_browser: F) -> Result<TokenSet>
    where
        F: FnOnce(String) -> Result<()>,
    {
        // Bind ephemeral or pinned port. We read the actual port back
        // out so the redirect_uri we register with the IdP matches
        // exactly what's listening — kernel-assigned port 0 returns
        // a real port, fixed_port returns itself.
        let bind_port = self.fixed_port.unwrap_or(0);
        let listener = TcpListener::bind(("127.0.0.1", bind_port))
            .await
            .map_err(|e| {
                CorivoError::Internal(if self.fixed_port.is_some() {
                    format!(
                        "OAuth loopback port {bind_port} is in use ({e}). \
                     Close other Corivo sign-in flows or apps using it and retry."
                    )
                } else {
                    format!("Could not bind local port: {e}")
                })
            })?;
        let port = listener
            .local_addr()
            .map_err(|e| CorivoError::Internal(format!("Could not read local port: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        let auth_url = AuthUrl::new(self.auth_url.clone())
            .map_err(|e| CorivoError::Internal(format!("auth url: {e}")))?;
        let token_url = TokenUrl::new(self.token_url.clone())
            .map_err(|e| CorivoError::Internal(format!("token url: {e}")))?;
        let redirect = RedirectUrl::new(redirect_uri.clone())
            .map_err(|e| CorivoError::Internal(format!("redirect url: {e}")))?;

        // We use BasicClient to mint the authorize URL + PKCE only.
        // The actual token exchange is a manual reqwest POST in
        // token::exchange_code so we can surface the full response
        // (id_token / refresh_token / scope) — oauth2's typed
        // BasicTokenResponse doesn't expose those.
        let client = BasicClient::new(oauth2::ClientId::new(self.client_id.clone()))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect);

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        let mut builder = client.authorize_url(CsrfToken::new_random);
        for scope in &self.scopes {
            builder = builder.add_scope(Scope::new(scope.clone()));
        }
        for (k, v) in &self.extra_auth_params {
            builder = builder.add_extra_param(k.as_str(), v.as_str());
        }
        let (authorize_url, csrf_state) = builder.set_pkce_challenge(pkce_challenge).url();

        open_browser(authorize_url.to_string())?;

        let (auth_code, returned_state) =
            server::wait_for_callback(&listener, &self.success_url, &self.error_url_prefix).await?;

        if returned_state != *csrf_state.secret() {
            return Err(CorivoError::Internal(
                "OAuth callback state mismatch (possible CSRF)".to_string(),
            ));
        }

        token::exchange_code(
            &self.token_url,
            &self.client_id,
            self.client_secret.as_deref(),
            &redirect_uri,
            &auth_code,
            pkce_verifier.secret(),
        )
        .await
    }

    /// Trade a `refresh_token` for a fresh access_token (and possibly
    /// a rotated refresh_token, though Google rarely rotates these).
    /// Used by `services::connector` when a cached access_token is
    /// approaching its TTL or returned 401.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet> {
        token::refresh_token(
            &self.token_url,
            &self.client_id,
            self.client_secret.as_deref(),
            refresh_token,
        )
        .await
    }
}
