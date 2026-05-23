//! Compile-time list of built-in connectors.
//!
//! Scope today: ONLY `cliInstall`-shape connectors (Lark CLI, GitHub CLI).
//! Every SaaS that used to live here as an OAuth or vendor-MCP connector
//! (Gmail / Google Docs / Google Calendar / Slack / Notion / Linear /
//! GitLab) has been pulled out — those now go through the Corivo backend's
//! Composio MCP gateway instead. See `apps/api/src/routes/composio.ts`.
//!
//! Why compile-time inline instead of runtime fs lookup: the Tauri crate
//! has no obvious place to read from at startup (the bundled `.app`
//! layout differs from the dev workspace layout). `include_str!` makes
//! the manifest part of the binary, with the manifest-versioning side
//! effect that "ship a new manifest" requires "ship a new binary" —
//! which is exactly what we want until CDN-distributed connectors land
//! in Phase 2.

use std::sync::OnceLock;

use crate::error::{CorivoError, Result};
use crate::services::connector::manifest::ConnectorManifest;

/// `packages/connector-feishu-cli/manifest.json` — `cliInstall` auth
/// shape: clicking 安装 in Settings → Integrations opens a fresh `/ask`
/// thread with a preset prompt that asks the agent to detect the
/// user's environment and install the `lark` CLI. 已安装 is derived
/// by probing `lark` on `PATH` (see `binary_on_path` in `auth.rs`).
const FEISHU_CLI_MANIFEST_JSON: &str =
    include_str!("../../../../../../packages/connector-feishu-cli/manifest.json");

/// `packages/connector-github-cli/manifest.json` — `cliInstall`-shape
/// connector wrapping the official `gh` CLI. Same pattern as feishu-cli:
/// 安装 opens a fresh `/ask` thread, agent verifies `gh` on PATH and
/// walks the user through `brew install gh` (or platform equivalent)
/// + `gh auth login`.
const GITHUB_CLI_MANIFEST_JSON: &str =
    include_str!("../../../../../../packages/connector-github-cli/manifest.json");

fn parse_builtin() -> Result<Vec<ConnectorManifest>> {
    let feishu_cli: ConnectorManifest = serde_json::from_str(FEISHU_CLI_MANIFEST_JSON)
        .map_err(|e| CorivoError::Internal(format!("builtin feishu-cli manifest parse: {e}")))?;
    let github_cli: ConnectorManifest = serde_json::from_str(GITHUB_CLI_MANIFEST_JSON)
        .map_err(|e| CorivoError::Internal(format!("builtin github-cli manifest parse: {e}")))?;
    Ok(vec![feishu_cli, github_cli])
}

/// Cached list — parsed once on first access. Failure to parse is a
/// programmer bug (manifest schema drift); we crash with a clear
/// message rather than returning empty + hiding it.
pub fn builtin_manifests() -> &'static [ConnectorManifest] {
    static CACHE: OnceLock<Vec<ConnectorManifest>> = OnceLock::new();
    CACHE.get_or_init(|| match parse_builtin() {
        Ok(v) => v,
        Err(e) => panic!("connector catalog: {e}"),
    })
}

/// Look up a manifest by id. Returns None for unknown ids — callers
/// should surface "connector not found" rather than 500.
pub fn manifest_for(id: &str) -> Option<&'static ConnectorManifest> {
    builtin_manifests().iter().find(|m| m.id == id)
}
