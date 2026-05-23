//! Connector-level OAuth orchestrator.
//!
//! Wraps `services::oauth_loopback` for the consumer side: pick the
//! right client_id/secret based on `manifest.auth.provider`, set the
//! browser-open closure, persist the resulting `ProviderAccount` (with
//! tokens) to `Config.connectors.accounts`.
//!
//! Provider-shared accounts (B model): a `ProviderAccount` row in
//! Config is keyed by `"<provider>:<account_id>"`. Connectors point at
//! it via `Config.connectors.bindings[connector_id]`. Three Google
//! connectors on the same Google account share one row + one refresh
//! schedule. Connecting a second Google connector when the user
//! already has a Google account bound: if the manifest's scopes are
//! already covered, no browser; otherwise OAuth with
//! `include_granted_scopes=true` + `login_hint=<email>` so consent is
//! one-click ("add Send email permission").
//!
//! Token storage: tokens live IN `Config.connectors.accounts[key]`
//! alongside the rest of the account metadata. We deliberately do NOT
//! use macOS Keychain — see the `ConnectorsConfig` docstring. The
//! refresh single-flight mutex (`refresh_locks`) is the only piece of
//! in-memory state we keep; the access_token + expires_at are read
//! straight from Config on every fetch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::domain::config::ProviderAccount;
use crate::env;
use crate::error::{CorivoError, Result};
use crate::services::config_service::ConfigService;
use crate::services::connector::{account_key, catalog, manifest::ConnectorAuthConfig};
use crate::services::connector::{ConnectorSummary, ConnectorTokenSnapshot};
use crate::services::oauth_loopback::{LoopbackFlow, TokenSet};

/// 5 minutes — same budget as the cloud auth login flow. If the user
/// hasn't completed consent in the browser by then, they almost
/// certainly bailed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

/// Refresh access_token if it expires within this window. Picked to be
/// long enough that we never hand out a token that dies mid-call, but
/// short enough that we don't refresh constantly during normal use.
const REFRESH_LEEWAY: chrono::Duration = chrono::Duration::seconds(60);

/// Pinned loopback port used by the Slack OAuth flow. Slack matches
/// `redirect_uri` against the URL set in app config exactly, so we
/// commit to one port and ask the user to register the matching URL
/// (`http://127.0.0.1:8989/callback`) once. See `LoopbackFlow::fixed_port`.
const SLACK_LOOPBACK_PORT: u16 = 8989;

#[derive(Debug, Error)]
pub enum ConnectorAuthError {
    #[error("unknown connector id: {0}")]
    UnknownId(String),
    #[error("OAuth provider {0} is not configured on this build")]
    ProviderNotConfigured(&'static str),
    #[error("connector {0} has no connected account")]
    NotConnected(String),
    #[error("refresh_token is missing or revoked for account {0}; user must reconnect")]
    NeedsReauth(String),
}

impl From<ConnectorAuthError> for CorivoError {
    fn from(e: ConnectorAuthError) -> Self {
        CorivoError::Internal(e.to_string())
    }
}

/// State backing `ConnectorRegistry`. Held inside `Arc` so handlers
/// + the agent runtime can clone cheaply.
pub struct ConnectorAuthCore {
    app: AppHandle,
    config: Arc<ConfigService>,
    /// account_key → per-account mutex; entries are created lazily.
    /// Single-flight guard for `refresh_token` exchange so a burst of
    /// agent fetches doesn't fire N parallel refresh POSTs (Google
    /// would 429 + we'd waste round-trips). Std Mutex around the map
    /// is OK — we only hold it long enough to `Arc::clone` the inner
    /// tokio Mutex; the latter is what holds across the await point.
    refresh_locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl ConnectorAuthCore {
    pub fn new(app: AppHandle, config: Arc<ConfigService>) -> Self {
        Self {
            app,
            config,
            refresh_locks: StdMutex::new(HashMap::new()),
        }
    }

    // ─────────────────────────────────────────── enable / disable

    pub fn list(&self) -> Vec<ConnectorSummary> {
        let cfg = self.config.get();
        catalog::builtin_manifests()
            .iter()
            .map(|m| {
                let cli_installed = match &m.auth {
                    ConnectorAuthConfig::CliInstall { binary, .. } => Some(binary_on_path(binary)),
                    ConnectorAuthConfig::OAuth2 { .. } | ConnectorAuthConfig::McpServer { .. } => {
                        None
                    }
                };
                ConnectorSummary {
                    id: m.id.clone(),
                    manifest: m.clone(),
                    enabled: cfg.connectors.enabled.iter().any(|id| id == &m.id),
                    account: resolve_bound_account(&cfg, &m.id),
                    cli_installed,
                }
            })
            .collect()
    }

    pub fn enable(&self, id: &str) -> Result<ConnectorSummary> {
        catalog::manifest_for(id).ok_or_else(|| ConnectorAuthError::UnknownId(id.to_string()))?;
        let mut cfg = self.config.get();
        if !cfg.connectors.enabled.iter().any(|x| x == id) {
            cfg.connectors.enabled.push(id.to_string());
            self.config.update(cfg)?;
        }
        Ok(self.summary_for(id))
    }

    pub fn disable(&self, id: &str) -> Result<()> {
        let mut cfg = self.config.get();
        cfg.connectors.enabled.retain(|x| x != id);
        self.config.update(cfg)?;
        Ok(())
    }

    fn summary_for(&self, id: &str) -> ConnectorSummary {
        let cfg = self.config.get();
        let manifest = catalog::manifest_for(id).expect("checked").clone();
        let cli_installed = match &manifest.auth {
            ConnectorAuthConfig::CliInstall { binary, .. } => Some(binary_on_path(binary)),
            ConnectorAuthConfig::OAuth2 { .. } | ConnectorAuthConfig::McpServer { .. } => None,
        };
        ConnectorSummary {
            id: id.to_string(),
            manifest,
            enabled: cfg.connectors.enabled.iter().any(|x| x == id),
            account: resolve_bound_account(&cfg, id),
            cli_installed,
        }
    }

    // ─────────────────────────────────────────── connect

    /// Connect a single connector. Thin wrapper around
    /// [`connect_provider`] for the legacy "click connect on a single
    /// row" flow — derives the provider from the manifest and runs the
    /// multi-id codepath with a one-element list.
    pub async fn connect(&self, id: &str) -> Result<ConnectorSummary> {
        let manifest = catalog::manifest_for(id)
            .ok_or_else(|| ConnectorAuthError::UnknownId(id.to_string()))?;
        let provider = match &manifest.auth {
            ConnectorAuthConfig::OAuth2 { provider, .. } => provider.clone(),
            ConnectorAuthConfig::CliInstall { .. } => {
                return Err(CorivoError::Internal(format!(
                    "connector {id} uses cliInstall, not OAuth — install via the /ask prompt instead"
                )));
            }
            ConnectorAuthConfig::McpServer { .. } => {
                return Err(CorivoError::Internal(format!(
                    "connector {id} uses mcpServer auth — use the MCP bootstrap path, not connect"
                )));
            }
        };
        self.connect_provider(&provider, &[id.to_string()])
            .await
            .map(|mut v| v.pop().unwrap_or_else(|| self.summary_for(id)))
    }

    /// Bulk connect: open ONE OAuth round-trip whose scope is the union
    /// of every listed connector's manifest scopes, then bind + enable
    /// all of them on the resulting account. This is what powers the
    /// Settings → Integrations "Connect Google (3 项)" button — checking
    /// gcal + gmail + google-docs and clicking once should open the
    /// browser exactly once, not three times.
    ///
    /// All `connector_ids` MUST share `manifest.auth.provider == provider`
    /// — mixing providers (asking to connect google + slack in one shot)
    /// is rejected. Empty `connector_ids` is an error too; nothing
    /// useful to do.
    pub async fn connect_provider(
        &self,
        provider: &str,
        connector_ids: &[String],
    ) -> Result<Vec<ConnectorSummary>> {
        if connector_ids.is_empty() {
            return Err(CorivoError::Internal(
                "connect_provider called with empty connector_ids".to_string(),
            ));
        }

        // Validate every id, harvest each manifest's scopes + extra
        // auth params. Bail loudly on mismatch — silently dropping
        // unknown ids would leave the user wondering why their
        // checkbox didn't take effect.
        let mut union_scopes_buf: Vec<String> = Vec::new();
        let mut extra_auth_params: HashMap<String, String> = HashMap::new();
        let mut auth_url = String::new();
        let mut token_url = String::new();
        for id in connector_ids {
            let manifest = catalog::manifest_for(id)
                .ok_or_else(|| ConnectorAuthError::UnknownId(id.to_string()))?;
            match &manifest.auth {
                ConnectorAuthConfig::OAuth2 {
                    provider: mp,
                    auth_url: au,
                    token_url: tu,
                    extra_auth_params: extras,
                    ..
                } => {
                    if mp != provider {
                        return Err(CorivoError::Internal(format!(
                            "connector {id} provider {mp:?} doesn't match requested {provider:?}"
                        )));
                    }
                    // Endpoints come from any one manifest — every
                    // connector on the same provider points at the
                    // same auth/token URL.
                    if auth_url.is_empty() {
                        auth_url = au.clone();
                        token_url = tu.clone();
                    }
                    for (k, v) in extras {
                        extra_auth_params
                            .entry(k.clone())
                            .or_insert_with(|| v.clone());
                    }
                    for s in manifest.all_scopes() {
                        if !union_scopes_buf.iter().any(|x| x == &s) {
                            union_scopes_buf.push(s);
                        }
                    }
                }
                ConnectorAuthConfig::CliInstall { .. } => {
                    return Err(CorivoError::Internal(format!(
                        "connector {id} uses cliInstall, not OAuth — install via the /ask prompt instead"
                    )));
                }
                ConnectorAuthConfig::McpServer { .. } => {
                    return Err(CorivoError::Internal(format!(
                        "connector {id} uses mcpServer auth — use the MCP bootstrap path, not connect"
                    )));
                }
            }
        }

        // ── Fast path: every requested scope is already covered by an
        // existing account on this provider. Bind + enable all
        // connectors silently, no browser. This is the path "user
        // toggles a Google product on AFTER having connected another"
        // takes — Google's `include_granted_scopes=true` made the
        // initial token already cover the new scope, so no work needed
        // beyond writing two HashMap entries.
        if let Some(existing) = find_provider_account(&self.config.get(), provider) {
            if !existing.needs_reauth
                && union_scopes_buf
                    .iter()
                    .all(|s| existing.granted_scopes.iter().any(|g| g == s))
            {
                let key = account_key(&existing.provider, &existing.account_id);
                let mut cfg = self.config.get();
                for id in connector_ids {
                    cfg.connectors.bindings.insert(id.clone(), key.clone());
                    if !cfg.connectors.enabled.iter().any(|x| x == id) {
                        cfg.connectors.enabled.push(id.clone());
                    }
                }
                self.config.update(cfg)?;
                return Ok(connector_ids
                    .iter()
                    .map(|id| self.summary_for(id))
                    .collect());
            }
        }

        // ── Slow path: run OAuth. If an account already exists for
        // this provider, pin the browser to it via `login_hint` so the
        // user doesn't accidentally consent on a different Google
        // account and produce a second `ProviderAccount` row.
        let existing_email =
            find_provider_account(&self.config.get(), provider).and_then(|a| a.email);
        if let Some(ref hint) = existing_email {
            extra_auth_params
                .entry("login_hint".to_string())
                .or_insert_with(|| hint.clone());
        }

        // Scope routing differs per provider:
        //   Google → identity baseline (openid+email+profile, for the
        //     id_token → sub/email/name/avatar) + manifest business
        //     scopes, all in the standard `scope=` param.
        //   Slack  → identity baseline is empty (no OIDC); manifest
        //     business scopes go into the Slack-specific `user_scope=`
        //     query param (comma-joined), NOT the standard `scope=`,
        //     so the response is a pure user-token flow (xoxp-…) with
        //     no bot side-effect.
        let identity_scopes = identity_scopes_for(provider);
        let mut scopes = identity_scopes.clone();
        if provider == "slack" {
            let user_scope = union_scopes_buf.join(",");
            if !user_scope.is_empty() {
                extra_auth_params.insert("user_scope".to_string(), user_scope);
            }
        } else {
            for s in &union_scopes_buf {
                if !scopes.iter().any(|x| x == s) {
                    scopes.push(s.clone());
                }
            }
        }

        let (client_id, client_secret) = client_credentials_for(provider)?;

        // Slack pins the loopback to a known port because its
        // `redirect_uri` policy is exact-match (no port wildcards).
        // Other providers (Google) accept any loopback port; leave them
        // ephemeral.
        let fixed_port = if provider == "slack" {
            Some(SLACK_LOOPBACK_PORT)
        } else {
            None
        };

        let flow = LoopbackFlow {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes,
            extra_auth_params: extra_auth_params.into_iter().collect(),
            success_url: format!("{}/oauth/success", env::web_base()),
            error_url_prefix: format!("{}/oauth/error", env::web_base()),
            fixed_port,
        };

        let app_for_browser = self.app.clone();
        let open_browser = move |url: String| {
            app_for_browser
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| CorivoError::Internal(format!("Could not open browser: {e}")))
        };

        let mut tokens = tokio::time::timeout(CONNECT_TIMEOUT, flow.run(open_browser))
            .await
            .map_err(|_| {
                CorivoError::Internal(format!("{provider} sign-in timed out (5 min)"))
            })??;

        // Slack stuffs user-token flow fields inside `authed_user`. The
        // rest of the connector pipeline (Keychain write, ctx.fetch
        // Bearer injection, 401 retry) expects them on the top-level
        // TokenSet, so lift them up before going further.
        if provider == "slack" {
            normalize_slack_token_set(&mut tokens)?;
        }

        let profile = ProviderProfile::from_token_set(provider, &tokens).await?;
        let key = account_key(provider, &profile.account_id);
        let expires_at = tokens.expires_at.map(|d| d.to_rfc3339());

        // Upsert account metadata + tokens in one Config write.
        // `granted_scopes` accumulates as a union so a second connector
        // on the same account widens the row's view of what it can do.
        let now = Utc::now().to_rfc3339();
        let mut cfg = self.config.get();
        let merged = match cfg.connectors.accounts.get(&key).cloned() {
            Some(prev) => ProviderAccount {
                provider: provider.to_string(),
                account_id: profile.account_id.clone(),
                email: profile.email.clone().or(prev.email),
                display_name: profile.name.clone().or(prev.display_name),
                avatar_url: profile.avatar_url.clone().or(prev.avatar_url),
                granted_scopes: union_scopes(&prev.granted_scopes, &tokens.scope),
                connected_at: prev.connected_at,
                last_refresh_at: prev.last_refresh_at,
                needs_reauth: false,
                access_token: tokens.access_token.clone().or(prev.access_token),
                // Google sometimes rotates the refresh_token; only
                // overwrite if the response actually has one.
                refresh_token: tokens.refresh_token.clone().or(prev.refresh_token),
                expires_at: expires_at.or(prev.expires_at),
            },
            None => ProviderAccount {
                provider: provider.to_string(),
                account_id: profile.account_id.clone(),
                email: profile.email.clone(),
                display_name: profile.name.clone(),
                avatar_url: profile.avatar_url.clone(),
                granted_scopes: tokens.scope.clone(),
                connected_at: now,
                last_refresh_at: None,
                needs_reauth: false,
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at,
            },
        };
        cfg.connectors.accounts.insert(key.clone(), merged);
        // Bind + enable every requested connector on this freshly
        // OAuth'd account.
        for id in connector_ids {
            cfg.connectors.bindings.insert(id.clone(), key.clone());
            if !cfg.connectors.enabled.iter().any(|x| x == id) {
                cfg.connectors.enabled.push(id.clone());
            }
        }
        self.config.update(cfg)?;

        // Bring the main window forward — user's eyes are still in the
        // browser when consent completes.
        if let Some(window) = self.app.get_webview_window("main") {
            let win = window.clone();
            let _ = self.app.run_on_main_thread(move || {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            });
        }

        Ok(connector_ids
            .iter()
            .map(|id| self.summary_for(id))
            .collect())
    }

    // ─────────────────────────────────────────── install (mcpServer)

    /// Run the sidecar's `--bootstrap-mcp-oauth` one-shot, then persist
    /// the synthetic account metadata that flips this connector to
    /// "connected". See [`super::mcp::bootstrap_install`].
    pub async fn install_mcp(&self, id: &str) -> Result<ConnectorSummary> {
        super::mcp::bootstrap_install(&self.app, &self.config, id).await?;

        // Bring the main window forward — user's focus is in the
        // browser when OAuth completes; we want to pull them back to
        // Corivo. Same UX as `connect()`.
        if let Some(window) = self.app.get_webview_window("main") {
            let win = window.clone();
            let _ = self.app.run_on_main_thread(move || {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            });
        }

        Ok(self.summary_for(id))
    }

    // ─────────────────────────────────────────── disconnect

    pub async fn disconnect(&self, id: &str) -> Result<ConnectorSummary> {
        // Branch first: mcpServer connectors don't have Keychain entries,
        // they have an on-disk mcporter token cache instead.
        let manifest = catalog::manifest_for(id);
        let is_mcp = matches!(
            manifest.map(|m| &m.auth),
            Some(ConnectorAuthConfig::McpServer { .. })
        );

        let mut cfg = self.config.get();
        cfg.connectors.enabled.retain(|x| x != id);
        let removed_key = cfg.connectors.bindings.remove(id);

        // Reference counting: only evict the account row (and its
        // tokens) when this connector was the last one bound to it.
        // Keeps gmail working when the user disconnects gcal but not
        // gmail.
        if let Some(key) = removed_key.as_deref() {
            let still_in_use = cfg.connectors.bindings.values().any(|k| k == key);
            if !still_in_use {
                cfg.connectors.accounts.remove(key);
                // (mcp is_mcp branch is a no-op here — `accounts.remove`
                // already drops the synthetic placeholder row. The
                // mcporter on-disk cache is wiped by `clear_oauth_caches`
                // below.)
                let _ = is_mcp;
            }
        }
        self.config.update(cfg)?;

        if is_mcp {
            super::mcp::clear_oauth_caches(&self.app, id).await;
        }

        Ok(self.summary_for(id))
    }

    // ─────────────────────────────────────────── access_token

    pub async fn get_access_token(&self, id: &str) -> Result<String> {
        let key = self
            .bound_account_key(id)
            .ok_or_else(|| ConnectorAuthError::NotConnected(id.to_string()))?;
        self.get_access_token_by_key(&key).await
    }

    async fn get_access_token_by_key(&self, key: &str) -> Result<String> {
        // Fast path: read directly from Config — `tauri-plugin-store`
        // keeps the latest snapshot in memory, so this is just a clone
        // of a HashMap entry. If the persisted token is fresh, return
        // it without acquiring the refresh lock.
        if let Some(account) = self.account_snapshot(key) {
            if let Some(token) = fresh_access_token(&account) {
                return Ok(token);
            }
        }

        // Slow path: single-flight refresh per account, so a burst of
        // agent fetches doesn't fire N parallel POSTs to the token
        // endpoint.
        let lock = self.lock_for(key);
        let _held = lock.lock().await;

        // Re-check after acquiring the lock — someone else may have
        // refreshed while we were waiting.
        if let Some(account) = self.account_snapshot(key) {
            if let Some(token) = fresh_access_token(&account) {
                return Ok(token);
            }
        }

        self.refresh_locked(key).await
    }

    fn bound_account_key(&self, id: &str) -> Option<String> {
        self.config.get().connectors.bindings.get(id).cloned()
    }

    fn account_snapshot(&self, key: &str) -> Option<ProviderAccount> {
        self.config.get().connectors.accounts.get(key).cloned()
    }

    /// Refresh assuming we hold `refresh_locks[key]`.
    async fn refresh_locked(&self, key: &str) -> Result<String> {
        let account = self
            .account_snapshot(key)
            .ok_or_else(|| ConnectorAuthError::NotConnected(key.to_string()))?;
        if account.needs_reauth {
            return Err(ConnectorAuthError::NeedsReauth(key.to_string()).into());
        }

        let refresh_token = account
            .refresh_token
            .clone()
            .ok_or_else(|| ConnectorAuthError::NeedsReauth(key.to_string()))?;

        // Find the OAuth endpoint URLs. Any connector on this provider
        // has the same auth_url/token_url, so just pick the first
        // manifest matching the provider — that's what dictates the
        // refresh round-trip's HTTP target.
        let (auth_url, token_url) = oauth_endpoints_for(&account.provider).ok_or_else(|| {
            CorivoError::Internal(format!(
                "no OAuth manifest known for provider {}",
                account.provider
            ))
        })?;
        let (client_id, client_secret) = client_credentials_for(&account.provider)?;

        let flow = LoopbackFlow {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes: vec![],
            extra_auth_params: vec![],
            success_url: String::new(),
            error_url_prefix: String::new(),
            // Refresh never binds the loopback (`refresh()` only POSTs
            // to the token endpoint), so `fixed_port` is irrelevant here.
            fixed_port: None,
        };

        match flow.refresh(&refresh_token).await {
            Ok(tokens) => self.apply_refresh(key, tokens).await,
            Err(e) => {
                // Heuristic: any 4xx-class refresh failure → mark
                // needs_reauth. Google's `invalid_grant` is the
                // common form. We don't have structured errors from
                // oauth_loopback today; the surfaced message embeds
                // the upstream body, which is enough to spot it.
                let msg = e.to_string();
                if msg.contains("invalid_grant") {
                    self.mark_needs_reauth(key);
                    return Err(ConnectorAuthError::NeedsReauth(key.to_string()).into());
                }
                Err(e)
            }
        }
    }

    async fn apply_refresh(&self, key: &str, tokens: TokenSet) -> Result<String> {
        let access = tokens
            .access_token
            .clone()
            .ok_or_else(|| CorivoError::Internal("refresh returned no access_token".to_string()))?;

        let expires_at = tokens.expires_at.map(|d| d.to_rfc3339());
        let mut cfg = self.config.get();
        if let Some(meta) = cfg.connectors.accounts.get_mut(key) {
            meta.access_token = Some(access.clone());
            // Google sometimes rotates — when present, replace; otherwise
            // keep the long-lived one we already have.
            if let Some(ref rt) = tokens.refresh_token {
                meta.refresh_token = Some(rt.clone());
            }
            meta.expires_at = expires_at;
            meta.last_refresh_at = Some(Utc::now().to_rfc3339());
            meta.needs_reauth = false;
        }
        self.config.update(cfg)?;

        Ok(access)
    }

    fn mark_needs_reauth(&self, key: &str) {
        let mut cfg = self.config.get();
        if let Some(meta) = cfg.connectors.accounts.get_mut(key) {
            meta.needs_reauth = true;
            // Wipe the dead access_token so a stale value can't get
            // handed back if `get_access_token` races with the user's
            // reconnect click. The refresh_token stays so the
            // upcoming reconnect can detect it as the same account.
            meta.access_token = None;
        }
        let _ = self.config.update(cfg);
    }

    fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self
            .refresh_locks
            .lock()
            .expect("connector refresh_locks mutex poisoned");
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    // ─────────────────────────────────────────── agent input snapshot

    pub async fn enabled_snapshot(&self) -> Vec<ConnectorTokenSnapshot> {
        let cfg = self.config.get();
        let ids: Vec<String> = cfg.connectors.enabled.clone();
        drop(cfg);

        let mut out = Vec::new();
        for id in ids {
            // Resolve connector → account via bindings. Connectors that
            // have been enabled but never connected won't have a
            // binding row yet, so they correctly fall out here.
            let Some(account_key) = self.bound_account_key(&id) else {
                continue;
            };
            let Some(account) = self.account_snapshot(&account_key) else {
                continue;
            };
            if account.needs_reauth {
                continue;
            }

            match self.get_access_token_by_key(&account_key).await {
                Ok(access_token) => {
                    // Re-read the account so `expires_at` reflects the
                    // refresh that may have just happened.
                    let post = self
                        .account_snapshot(&account_key)
                        .unwrap_or_else(|| account.clone());
                    out.push(ConnectorTokenSnapshot {
                        id: id.clone(),
                        account_email: post.email.clone(),
                        granted_scopes: post.granted_scopes.clone(),
                        access_token,
                        expires_at: post.expires_at.clone(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "connector",
                        error = %e,
                        connector_id = %id,
                        account_key = %account_key,
                        "connector.snapshot.get_token_failed",
                    );
                }
            }
        }
        out
    }
}

/// Return a fresh access_token for `account` if there is one and it's
/// not within `REFRESH_LEEWAY` of expiry. `None` otherwise → caller
/// must refresh.
fn fresh_access_token(account: &ProviderAccount) -> Option<String> {
    let token = account.access_token.as_ref()?;
    if let Some(ref iso) = account.expires_at {
        // If we have an expiry and we're inside the leeway, force refresh.
        if let Ok(parsed) = DateTime::parse_from_rfc3339(iso) {
            let when = parsed.with_timezone(&Utc);
            if Utc::now() + REFRESH_LEEWAY >= when {
                return None;
            }
        }
        // Unparseable expiry → safest to refresh.
        if DateTime::parse_from_rfc3339(iso).is_err() {
            return None;
        }
    }
    // No expiry info → IdP didn't tell us; trust the stored token and
    // let any 401 from the consumer side surface naturally.
    Some(token.clone())
}

/// Resolve a connector id to its bound `ProviderAccount`, or `None` if
/// no binding exists (or the binding points at a stale account_key).
fn resolve_bound_account(cfg: &crate::domain::config::Config, id: &str) -> Option<ProviderAccount> {
    let key = cfg.connectors.bindings.get(id)?;
    cfg.connectors.accounts.get(key).cloned()
}

/// First account belonging to `provider` we find in Config. Used by
/// `connect()` to decide between fast-path-bind, incremental-OAuth, or
/// full new login.
///
/// Multi-account-per-provider is a future extension — for now there's
/// at most one account per provider, so "first" is also "only".
fn find_provider_account(
    cfg: &crate::domain::config::Config,
    provider: &str,
) -> Option<ProviderAccount> {
    cfg.connectors
        .accounts
        .values()
        .find(|a| a.provider == provider)
        .cloned()
}

/// Merge `next` scopes into `prev`, preserving insertion order and
/// deduplicating. Used to grow a `ProviderAccount.granted_scopes` as
/// additional connectors connect on the same account.
fn union_scopes(prev: &[String], next: &[String]) -> Vec<String> {
    let mut out: Vec<String> = prev.to_vec();
    for s in next {
        if !out.iter().any(|x| x == s) {
            out.push(s.clone());
        }
    }
    out
}

/// Pull `(auth_url, token_url)` for a given provider by sampling the
/// first OAuth2 manifest in the catalog whose `provider` matches. Every
/// Google connector points at the same endpoints (and ditto Slack), so
/// this is well-defined even with one connector connected.
fn oauth_endpoints_for(provider: &str) -> Option<(String, String)> {
    for m in catalog::builtin_manifests() {
        if let ConnectorAuthConfig::OAuth2 {
            provider: p,
            auth_url,
            token_url,
            ..
        } = &m.auth
        {
            if p == provider {
                return Some((auth_url.clone(), token_url.clone()));
            }
        }
    }
    None
}

/// Probe whether `name` resolves to an executable on the user's `PATH`.
/// Used by `cliInstall` connectors to render the 已安装/未安装 badge in
/// Settings → Integrations.
///
/// Empty `PATH`, missing files, and unreadable directories all read as
/// "not installed" — we never want to surface a Rust IO error to the
/// settings UI because the user can't act on one.
fn binary_on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        let Ok(meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 != 0 {
                return true;
            }
        }
        #[cfg(not(unix))]
        {
            return true;
        }
    }
    false
}

/// Resolve OAuth client credentials based on the manifest's provider
/// field. Extending = adding a new arm here + matching env vars in
/// `env.rs`. Note: secret-required-ness varies by provider — Google
/// "Desktop app" clients treat it as optional, Slack rejects token
/// exchange without it.
fn client_credentials_for(provider: &str) -> Result<(String, Option<String>)> {
    match provider {
        "google" => {
            let id = env::google_client_id().ok_or_else(|| {
                CorivoError::Internal(
                    "Google OAuth client not configured: missing CORIVO_GOOGLE_CLIENT_ID"
                        .to_string(),
                )
            })?;
            Ok((id, env::google_client_secret()))
        }
        "slack" => {
            let id = env::slack_client_id().ok_or_else(|| {
                CorivoError::Internal(
                    "Slack OAuth client not configured: missing CORIVO_SLACK_CLIENT_ID".to_string(),
                )
            })?;
            // Slack v2 token exchange returns `invalid_client` without
            // the secret. Treat its absence as a hard config error
            // rather than passing None into the flow and getting a
            // cryptic upstream rejection.
            let secret = env::slack_client_secret().ok_or_else(|| {
                CorivoError::Internal(
                    "Slack OAuth client not configured: missing CORIVO_SLACK_CLIENT_SECRET"
                        .to_string(),
                )
            })?;
            Ok((id, Some(secret)))
        }
        _ => Err(ConnectorAuthError::ProviderNotConfigured("unknown").into()),
    }
}

/// Identity scopes we tack onto every connect for a given provider, so
/// we can derive a stable account id + UI metadata even when the
/// connector itself only requests business scopes (e.g. `gmail.send`).
///
/// Slack has no OIDC; we recover identity by calling `users.info` with
/// the freshly-issued user token (no extra scope required — the user's
/// own profile is always readable from their own token).
fn identity_scopes_for(provider: &str) -> Vec<String> {
    match provider {
        "google" => vec![
            "openid".to_string(),
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        ],
        _ => vec![],
    }
}

/// Lift Slack's user-token-flow fields out of `authed_user` into the
/// top-level `TokenSet`, so downstream code (Keychain writes, cached
/// fetch, refresh-on-401) treats Slack like any other OAuth provider.
///
/// Slack's v2 token response shape when only `user_scope=` is requested:
/// ```json
/// {
///   "ok": true,
///   "scope": "",
///   "access_token": null,                     // bot token, unused
///   "team":         { "id": "T...", "name": "..." },
///   "authed_user":  {
///     "id":            "U...",
///     "scope":         "chat:write,channels:read",  // comma-sep!
///     "access_token":  "xoxp-…",              // ← what we want
///     "token_type":    "user",
///     "refresh_token": "xoxe-…",              // present iff rotation on
///     "expires_in":    43200                   // present iff rotation on
///   }
/// }
/// ```
///
/// Returns Err if there's no `authed_user.access_token` — that means
/// the user consented to bot scopes instead of user scopes (which
/// shouldn't happen given we only send `user_scope=`), so failing
/// loud is more useful than silently saving an empty token.
fn normalize_slack_token_set(tokens: &mut TokenSet) -> Result<()> {
    let raw = tokens.raw_response.as_ref().ok_or_else(|| {
        CorivoError::Internal("Slack token response was not valid JSON".to_string())
    })?;

    let authed_user = raw.get("authed_user").ok_or_else(|| {
        CorivoError::Internal(
            "Slack token response missing `authed_user`; check that user_scope was requested"
                .to_string(),
        )
    })?;

    let user_access = authed_user
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| {
            CorivoError::Internal(
                "Slack token response missing `authed_user.access_token`".to_string(),
            )
        })?;
    tokens.access_token = Some(user_access);

    if let Some(rt) = authed_user.get("refresh_token").and_then(|v| v.as_str()) {
        tokens.refresh_token = Some(rt.to_string());
    }

    if let Some(exp) = authed_user.get("expires_in").and_then(|v| v.as_i64()) {
        if let Some(d) = chrono::Duration::try_seconds(exp) {
            tokens.expires_at = Some(Utc::now() + d);
        }
    }

    // Slack's `authed_user.scope` is comma-separated, unlike the
    // OAuth-standard space-separated `scope` the generic parser
    // expects. Re-split it so granted_scopes reads cleanly downstream.
    if let Some(scope_str) = authed_user.get("scope").and_then(|v| v.as_str()) {
        let split: Vec<String> = scope_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !split.is_empty() {
            tokens.scope = split;
        }
    }

    Ok(())
}

// ─────────────────────────────────────────── id_token claim parsing

#[derive(Debug, Clone)]
struct ProviderProfile {
    account_id: String,
    email: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

impl ProviderProfile {
    async fn from_token_set(provider: &str, tokens: &TokenSet) -> Result<Self> {
        match provider {
            "google" => Self::from_google_id_token(tokens),
            "slack" => Self::from_slack_token_response(tokens).await,
            _ => Err(ConnectorAuthError::ProviderNotConfigured("unknown").into()),
        }
    }

    /// Decode the OIDC id_token payload (we do NOT verify the signature
    /// here — Google's TLS to `oauth2.googleapis.com` plus our PKCE
    /// `state` check is already the chain of trust, and we only use
    /// claims for client-side metadata, not authorization).
    fn from_google_id_token(tokens: &TokenSet) -> Result<Self> {
        let id_token = tokens.id_token.as_deref().ok_or_else(|| {
            CorivoError::Internal(
                "Google OAuth response is missing id_token; check that openid scope is requested"
                    .to_string(),
            )
        })?;
        let payload = id_token
            .split('.')
            .nth(1)
            .ok_or_else(|| CorivoError::Internal("malformed id_token (no payload)".to_string()))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.as_bytes())
            .map_err(|e| CorivoError::Internal(format!("id_token base64 decode: {e}")))?;
        let claims: GoogleIdTokenClaims = serde_json::from_slice(&bytes)
            .map_err(|e| CorivoError::Internal(format!("id_token JSON parse: {e}")))?;
        Ok(Self {
            account_id: claims.sub,
            email: claims.email,
            name: claims.name,
            avatar_url: claims.picture,
        })
    }

    /// Slack identity recovery: there's no id_token, so we read the
    /// team + user ids out of the token response (already normalized
    /// by `normalize_slack_token_set`) and then call `users.info` to
    /// hydrate display name, email, avatar.
    ///
    /// `account_id` is `<team_id>:<user_id>` so the same Slack user
    /// across two workspaces lands in two distinct Corivo accounts
    /// (which they are, from Slack's API perspective).
    ///
    /// We do NOT fail the connect if `users.info` returns an error
    /// (e.g. `missing_scope` if the user didn't grant `users:read`) —
    /// the OAuth flow already succeeded; we just lose the email /
    /// avatar UI hint and proceed with team_id:user_id as identity.
    async fn from_slack_token_response(tokens: &TokenSet) -> Result<Self> {
        let raw = tokens.raw_response.as_ref().ok_or_else(|| {
            CorivoError::Internal("Slack token response was not valid JSON".to_string())
        })?;

        let team_id = raw
            .get("team")
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CorivoError::Internal("Slack token response missing `team.id`".to_string())
            })?;
        let user_id = raw
            .get("authed_user")
            .and_then(|u| u.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CorivoError::Internal("Slack token response missing `authed_user.id`".to_string())
            })?;
        let user_token = tokens.access_token.as_deref().ok_or_else(|| {
            CorivoError::Internal(
                "Slack token response missing user access_token after normalization".to_string(),
            )
        })?;

        let (email, name, avatar_url) = fetch_slack_user_profile(user_id, user_token).await;

        Ok(Self {
            account_id: format!("{team_id}:{user_id}"),
            email,
            name,
            avatar_url,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GoogleIdTokenClaims {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
}

/// Best-effort fetch of `users.info` for display name / email / avatar.
/// Returns `(None, None, None)` on any failure — we never let a
/// profile-decoration miss break the OAuth flow.
async fn fetch_slack_user_profile(
    user_id: &str,
    user_token: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                target: "connector",
                error = %e,
                "slack.users_info.http_client_failed",
            );
            return (None, None, None);
        }
    };

    let response = match http
        .get("https://slack.com/api/users.info")
        .bearer_auth(user_token)
        .query(&[("user", user_id)])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "connector",
                error = %e,
                "slack.users_info.network_failed",
            );
            return (None, None, None);
        }
    };

    let body: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "connector",
                error = %e,
                "slack.users_info.json_failed",
            );
            return (None, None, None);
        }
    };

    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            target: "connector",
            slack_error = err,
            "slack.users_info.not_ok",
        );
        return (None, None, None);
    }

    let profile = body.get("user").and_then(|u| u.get("profile"));
    let email = profile
        .and_then(|p| p.get("email"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let name = profile
        .and_then(|p| p.get("real_name").or_else(|| p.get("display_name")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let avatar_url = profile
        .and_then(|p| {
            p.get("image_192")
                .or_else(|| p.get("image_72"))
                .or_else(|| p.get("image_48"))
        })
        .and_then(|v| v.as_str())
        .map(String::from);
    (email, name, avatar_url)
}
