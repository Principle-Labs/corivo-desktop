//! Connector framework — uniform OAuth + token management + agent
//! exposure for third-party services (Gmail, future: Notion, Slack, …).
//!
//! Design notes (see `apps/desktop/docs/connector-framework-spec.md`):
//!
//! * Each connector ships as a separate workspace package
//!   (`packages/connector-<id>/`) with a hand-edited `manifest.json`
//!   describing OAuth config + display strings, plus a Bun-built
//!   bundle exporting `createTools(ctx) -> AgentTool[]`.
//! * The Rust side here is the framework — it knows nothing about
//!   Gmail specifically. The list of built-in connectors lives in
//!   `catalog.rs`; adding a new connector at the framework level is
//!   "add a `include_str!` of the new manifest.json".
//! * OAuth flow reuses `services::oauth_loopback`. Tokens live alongside
//!   account metadata in `Config.connectors.accounts[key].{access_token,
//!   refresh_token, expires_at}` (plaintext in `config.json` via
//!   `tauri-plugin-store` — same trust model as the Anthropic API key
//!   and Corivo session token already in `Config`). We deliberately do
//!   NOT use macOS Keychain; see the [`crate::domain::config::ConnectorsConfig`]
//!   docstring for the why.
//! * The agent sidecar receives a per-turn snapshot
//!   `{ id, access_token, expires_at }[]` via input JSON. Refresh
//!   during a long turn is brokered over the existing stdio NDJSON
//!   channel (see `runtime.rs`, wired in `services::exec_agent::runner`).

pub mod auth;
pub mod catalog;
pub mod manifest;
pub mod mcp;
pub mod runtime;

pub use auth::ConnectorAuthError;
pub use manifest::{
    ConnectorAuthConfig, ConnectorManifest, ConnectorOAuth2Scope, LocalizedString, McpTransport,
};
pub use runtime::ConnectorTokenSnapshot;

use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;
use ts_rs::TS;

use crate::domain::config::ProviderAccount;
use crate::error::Result;
use crate::services::config_service::ConfigService;

/// Canonical `"<provider>:<account_id>"` key used everywhere a connector
/// binding needs to point at a `ProviderAccount`. Centralised so the
/// format never drifts between writers (auth.rs) and readers (mcp.rs,
/// snapshot pipeline, settings UI).
pub fn account_key(provider: &str, account_id: &str) -> String {
    format!("{provider}:{account_id}")
}

/// One row of the Settings → Integrations list. Returned by
/// `connectors_list` Tauri command.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSummary {
    pub id: String,
    pub manifest: ConnectorManifest,
    /// True iff `id` is in `Config.connectors.enabled`.
    pub enabled: bool,
    /// Resolved through `Config.connectors.bindings[id]`. Present iff a
    /// binding exists and points at a known `accounts` row. Multiple
    /// connectors can surface the same `ProviderAccount` value — the
    /// UI uses this to render the same email/avatar on every Google
    /// connector card after one OAuth.
    pub account: Option<ProviderAccount>,
    /// Only meaningful for `cliInstall` connectors: `Some(true)` if
    /// the manifest's `binary` is on the user's `PATH`, `Some(false)`
    /// if not. `None` for OAuth-style connectors (where 已安装/未安装
    /// isn't the right axis — they use `account` + `enabled` instead).
    pub cli_installed: Option<bool>,
}

/// The framework's main façade. One instance lives on `AppState`,
/// owns the in-memory token cache + Keychain handle + manifest catalog.
pub struct ConnectorRegistry {
    inner: Arc<auth::ConnectorAuthCore>,
}

impl ConnectorRegistry {
    pub fn new(app: AppHandle, config: Arc<ConfigService>) -> Self {
        let inner = Arc::new(auth::ConnectorAuthCore::new(app, config));
        Self { inner }
    }

    /// Snapshot of every built-in connector + its current state.
    pub fn list(&self) -> Vec<ConnectorSummary> {
        self.inner.list()
    }

    /// Append `id` to `Config.connectors.enabled` (idempotent). Does
    /// NOT trigger OAuth — the user clicks "Connect" separately.
    pub fn enable(&self, id: &str) -> Result<ConnectorSummary> {
        self.inner.enable(id)
    }

    /// Remove `id` from `Config.connectors.enabled`. Keeps the account
    /// row + Keychain tokens so re-enabling restores the connection
    /// without re-consent.
    pub fn disable(&self, id: &str) -> Result<()> {
        self.inner.disable(id)
    }

    /// Run OAuth loopback flow for `id`. Persists tokens to Keychain
    /// and account metadata to Config. Idempotent: a second call for
    /// an already-connected connector re-runs consent and replaces
    /// the stored tokens (useful for switching accounts).
    pub async fn connect(&self, id: &str) -> Result<ConnectorSummary> {
        self.inner.connect(id).await
    }

    /// Multi-connector connect on a single provider. Opens the browser
    /// at most once: the OAuth `scope=` is the union of every listed
    /// connector's manifest scopes, and on success all of them are
    /// bound + enabled atomically. The Settings → Integrations
    /// `ProviderCard` calls this when the user clicks
    /// "Connect Google (3 项)".
    pub async fn connect_provider(
        &self,
        provider: &str,
        connector_ids: &[String],
    ) -> Result<Vec<ConnectorSummary>> {
        self.inner.connect_provider(provider, connector_ids).await
    }

    /// Install + authorize an `mcpServer`-shape connector. Spawns the
    /// sidecar in `--bootstrap-mcp-oauth` mode, which opens the
    /// browser, captures the OAuth callback, and writes the token to
    /// `$APPDATA/corivo-mcp-tokens/<id>/`. On success marks the
    /// connector as enabled + connected so the next agent turn picks
    /// it up via `runner/corivo.rs`.
    pub async fn install_mcp(&self, id: &str) -> Result<ConnectorSummary> {
        self.inner.install_mcp(id).await
    }

    /// Revoke local state: clear Keychain entries + drop the account
    /// row. Best-effort token revocation upstream (where supported)
    /// happens too, but failure there doesn't block local cleanup.
    /// For `mcpServer` connectors, also wipes the on-disk token cache
    /// so a re-install re-runs OAuth.
    pub async fn disconnect(&self, id: &str) -> Result<ConnectorSummary> {
        self.inner.disconnect(id).await
    }

    /// Fetch a current access_token, refreshing if near-expiry.
    /// Single-flight per connector. Called by `runtime.rs` when the
    /// agent sidecar asks via stdio mid-turn.
    pub async fn get_access_token(&self, id: &str) -> Result<String> {
        self.inner.get_access_token(id).await
    }

    /// What goes into agent input JSON at spawn time. Only includes
    /// enabled connectors that have a valid account.
    pub async fn enabled_snapshot(&self) -> Vec<ConnectorTokenSnapshot> {
        self.inner.enabled_snapshot().await
    }
}
