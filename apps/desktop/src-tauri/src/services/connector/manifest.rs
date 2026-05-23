//! Connector manifest schema. Hand-written `manifest.json` lives in
//! each `packages/connector-<id>/` package; we deserialize the file at
//! Rust crate compile time (see `catalog.rs` `include_str!`).
//!
//! These types are also exported to the frontend via ts-rs so the
//! Settings → Integrations UI can render localized labels straight
//! from the manifest without re-typing them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `{ "zh": "…", "en": "…" }`. Used for connector display name,
/// description, and per-scope labels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct LocalizedString {
    pub zh: String,
    pub en: String,
}

/// Top-level shape of `packages/connector-<id>/manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManifest {
    /// Stable lowercase id, matches the package directory name suffix
    /// (`packages/connector-<id>/`).
    pub id: String,
    pub name: LocalizedString,
    pub description: LocalizedString,
    pub version: String,
    /// Icon path relative to the connector package root. Frontend
    /// resolves via a Tauri resource (Phase 2 work). Phase 1 the
    /// frontend just shows the connector id letter as a fallback.
    pub icon: String,
    pub auth: ConnectorAuthConfig,
}

/// Tagged-union for future-proofing — `oauth2` is the original Phase 1
/// install method; `cliInstall` (added with the 飞书 CLI app) covers
/// connectors that don't have a server-side OAuth flow and instead
/// rely on a locally-installed binary the agent shells out to.
///
/// `rename_all = "camelCase"` lives on each variant (NOT the enum)
/// because serde's enum-level `rename_all` only renames variant
/// identifiers, not the fields inside variants. The manifest.json
/// uses `authUrl`/`tokenUrl`/`extraAuthParams`; without these
/// per-variant attrs serde would look for the snake_case Rust names
/// and fail to parse the manifest at boot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(tag = "type")]
pub enum ConnectorAuthConfig {
    #[serde(rename = "oauth2", rename_all = "camelCase")]
    #[ts(rename = "oauth2")]
    OAuth2 {
        /// Which Corivo-managed OAuth client to use. Phase 1 only
        /// understands `"google"` — extending to `"slack"`, `"notion"`
        /// etc. = adding env-var wiring in `services::connector::auth`.
        provider: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<ConnectorOAuth2Scope>,
        /// `access_type=offline`, `prompt=consent`,
        /// `include_granted_scopes=true` for Google. Empty for IdPs
        /// that don't need them.
        #[serde(default)]
        extra_auth_params: HashMap<String, String>,
    },
    /// "Install a CLI tool" connector — the UI shows an `安装` button
    /// that prefills a preset prompt into a fresh `/ask` thread; the
    /// agent then detects the user's OS / package manager and installs
    /// the binary on their behalf. "已安装" is derived by probing
    /// `binary` against `PATH` (see `binary_on_path` in `auth.rs`).
    /// There is no OAuth, no token, no `ProviderAccount` — the agent
    /// just shells out to the binary directly.
    #[serde(rename = "cliInstall", rename_all = "camelCase")]
    #[ts(rename = "cliInstall")]
    CliInstall {
        /// Binary name to probe on `PATH` for the "已安装" check
        /// (e.g. `"lark"` for the 飞书 CLI).
        binary: String,
        /// Preset prompt prefilled into the new `/ask` composer when
        /// the user clicks 安装. Localized so the agent answers in
        /// the user's language.
        install_prompt: LocalizedString,
    },
    /// Vendor-hosted MCP server connector — Corivo doesn't implement
    /// any tools, it just plugs the vendor's MCP endpoint into the
    /// sidecar (via mcporter). OAuth is handled by the MCP server
    /// itself per MCP spec dynamic-client-registration; the sidecar's
    /// mcporter runtime opens the browser, listens on a loopback
    /// callback, and caches the token under a Corivo-controlled
    /// `tokenCacheDir`. v0 only supports `transport: "http"` (SSE) —
    /// stdio (community / self-hosted servers) lands later.
    #[serde(rename = "mcpServer", rename_all = "camelCase")]
    #[ts(rename = "mcpServer")]
    McpServer {
        /// v0 only accepts `"http"`. `"stdio"` is reserved for a future
        /// pass; parsing it now returns an error so manifests don't
        /// silently ship in a half-supported state.
        transport: McpTransport,
        /// MCP endpoint URL (e.g. `https://mcp.linear.app/sse`).
        /// Required for `transport: "http"`.
        url: String,
    },
}

/// Wire-level enum for the MCP transport. `stdio` is reserved.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub enum McpTransport {
    #[serde(rename = "http")]
    #[ts(rename = "http")]
    Http,
    #[serde(rename = "stdio")]
    #[ts(rename = "stdio")]
    Stdio,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorOAuth2Scope {
    /// Full OAuth scope URL (`https://www.googleapis.com/auth/gmail.send`).
    pub value: String,
    /// Human-readable label for the consent / settings UI.
    pub label: LocalizedString,
    /// If false, the user can de-select this scope on the consent
    /// screen and we still treat the connection as successful (with
    /// the corresponding tool de-registered).
    pub required: bool,
}

impl ConnectorManifest {
    /// Convenience accessor for the OAuth2 variant — returns `None`
    /// for non-OAuth manifests.
    pub fn oauth2(&self) -> Option<&ConnectorAuthConfig> {
        match &self.auth {
            ConnectorAuthConfig::OAuth2 { .. } => Some(&self.auth),
            ConnectorAuthConfig::CliInstall { .. } | ConnectorAuthConfig::McpServer { .. } => None,
        }
    }

    pub fn required_scopes(&self) -> Vec<String> {
        match &self.auth {
            ConnectorAuthConfig::OAuth2 { scopes, .. } => scopes
                .iter()
                .filter(|s| s.required)
                .map(|s| s.value.clone())
                .collect(),
            ConnectorAuthConfig::CliInstall { .. } | ConnectorAuthConfig::McpServer { .. } => {
                Vec::new()
            }
        }
    }

    /// All requested scopes (required + optional). Passed to the
    /// loopback flow so the consent screen offers the full menu.
    pub fn all_scopes(&self) -> Vec<String> {
        match &self.auth {
            ConnectorAuthConfig::OAuth2 { scopes, .. } => {
                scopes.iter().map(|s| s.value.clone()).collect()
            }
            ConnectorAuthConfig::CliInstall { .. } | ConnectorAuthConfig::McpServer { .. } => {
                Vec::new()
            }
        }
    }
}
