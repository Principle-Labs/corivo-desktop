//! MCP-shape connector helpers.
//!
//! `mcpServer`-shape connectors (e.g. Linear) don't go through
//! `oauth_loopback` like Gmail does. Instead they delegate OAuth to
//! the sidecar's `mcporter` runtime — Corivo just spawns a one-shot
//! `corivo-agent --bootstrap-mcp-oauth <input>` child, which connects
//! to the MCP server, lets `mcporter` open the browser + capture the
//! callback, and writes the token to `token_cache_dir`. Subsequent
//! normal-agent invocations reconnect against the same cache directory
//! without re-prompting.
//!
//! Layering note: this module owns the *spawn* of the bootstrap child
//! plus the path scheme for token caches. The actual OAuth logic lives
//! in the sidecar (`packages/agent/src/mcp/bootstrap.ts`) — Rust doesn't
//! need a `keyring`-style codepath here because mcporter persists tokens
//! to disk on its own.

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncReadExt;

use crate::domain::config::ProviderAccount;
use crate::error::{CorivoError, Result};
use crate::services::cloud::connectors::ConnectorsService;
use crate::services::config_service::ConfigService;
use crate::services::connector::{account_key, catalog, manifest::ConnectorAuthConfig};
use crate::services::exec_agent::runner::corivo as runner;

/// Match `connect()`'s OAuth budget for visual consistency: the user
/// gets the same 5-minute window to complete consent regardless of
/// whether the connector is OAuth2 or mcpServer.
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(300);

/// Filesystem layout: `$APPDATA/corivo-mcp-tokens/<id>/`.
/// One directory per connector id; mcporter manages whatever it wants
/// inside (currently a JSON token file + DCR client info).
///
/// Why under `app_data_dir` and not the user's HOME: Corivo-owned
/// cleanup. `disconnect` wipes the whole directory; we don't want to
/// step on anything mcporter might have written for the user's other
/// editors (Cursor / Claude Desktop / …) which live under `~/.mcporter/`.
pub fn token_cache_dir(app: &AppHandle, id: &str) -> Result<PathBuf> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| CorivoError::Internal(format!("app_data_dir failed: {e}")))?;
    Ok(data.join("corivo-mcp-tokens").join(id))
}

/// JSON payload shape for the bootstrap child. Mirrors
/// `packages/agent/src/mcp/bootstrap.ts::BootstrapInput`.
fn bootstrap_payload(app: &AppHandle, id: &str, url: &str) -> Result<serde_json::Value> {
    let dir = token_cache_dir(app, id)?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        CorivoError::Internal(format!(
            "create token cache dir failed ({}): {e}",
            dir.display()
        ))
    })?;

    Ok(json!({
        "server": {
            "name": id,
            "transport": "http",
            "url": url,
            "token_cache_dir": dir.to_string_lossy(),
        },
        "timeout_ms": BOOTSTRAP_TIMEOUT.as_millis() as u64,
    }))
}

/// Spawn the sidecar in `--bootstrap-mcp-oauth` mode for `id`. Waits
/// for the child to exit; on success writes a synthetic
/// `ProviderAccount` (with `provider = "mcp"`) to Config so the UI knows the connector is
/// "connected" without needing per-vendor email/profile data (vendor
/// MCP servers don't surface that to the client).
///
/// Errors out if:
///   - `id` is unknown or the manifest isn't an `mcpServer` shape.
///   - The sidecar binary can't be located.
///   - The bootstrap child exits non-zero or times out (5 min).
pub async fn bootstrap_install(app: &AppHandle, config: &ConfigService, id: &str) -> Result<()> {
    let manifest = catalog::manifest_for(id)
        .ok_or_else(|| CorivoError::Internal(format!("connector {id} not found in catalog")))?;

    let url = match &manifest.auth {
        ConnectorAuthConfig::McpServer { transport, url } => {
            // v0: only http transport. The manifest schema in
            // `manifest.rs` already enumerates stdio, but the
            // bootstrap path here doesn't support it yet — refuse
            // loudly rather than silently producing a broken cache.
            if !matches!(
                transport,
                crate::services::connector::manifest::McpTransport::Http
            ) {
                return Err(CorivoError::Internal(format!(
                    "connector {id}: stdio MCP transport is not supported in v0"
                )));
            }
            url.clone()
        }
        _ => {
            return Err(CorivoError::Internal(format!(
                "connector {id} is not an mcpServer-shape connector"
            )));
        }
    };

    let payload = bootstrap_payload(app, id, &url)?;
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let input_path =
        std::env::temp_dir().join(format!("corivo-mcp-bootstrap-{id}-{pid}-{nanos}.json"));
    std::fs::write(&input_path, serde_json::to_vec(&payload)?).map_err(|e| {
        CorivoError::Internal(format!(
            "write bootstrap input failed ({}): {e}",
            input_path.display()
        ))
    })?;

    let launch = runner::locate_sidecar()?;
    let mut cmd = launch.command_with_args([
        std::ffi::OsStr::new("--bootstrap-mcp-oauth"),
        input_path.as_os_str(),
    ]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    tracing::info!(
        connector_id = %id,
        sidecar = %launch.trace_label(),
        "connector.mcp.bootstrap.spawn",
    );

    let mut child = cmd.spawn().map_err(|e| {
        CorivoError::Internal(format!(
            "spawn mcp bootstrap failed ({}): {e}",
            launch.trace_label()
        ))
    })?;

    let stderr = child.stderr.take();
    let stderr_task = stderr.map(|mut s| {
        let connector_id = id.to_string();
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(8 * 1024);
            let _ = s.read_to_end(&mut buf).await;
            if !buf.is_empty() {
                let text = String::from_utf8_lossy(&buf);
                for line in text.lines() {
                    if !line.is_empty() {
                        tracing::info!(
                            target: "corivo_agent",
                            connector_id = %connector_id,
                            "{line}",
                        );
                    }
                }
            }
        })
    });

    let status = tokio::time::timeout(BOOTSTRAP_TIMEOUT, child.wait())
        .await
        .map_err(|_| {
            CorivoError::Internal(format!(
                "{id} MCP OAuth timed out after {}s",
                BOOTSTRAP_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| CorivoError::Internal(format!("mcp bootstrap wait failed: {e}")))?;

    if let Some(t) = stderr_task {
        let _ = t.await;
    }
    let _ = std::fs::remove_file(&input_path);

    if !status.success() {
        return Err(CorivoError::Internal(format!(
            "{id} MCP OAuth child exited with {status}",
        )));
    }

    // Mark the connector as connected. mcpServer accounts don't carry
    // an email/profile (MCP spec doesn't expose user identity), so we
    // write the minimal placeholder so the UI's `account.is_some`
    // check flips to true and the runner picks the connector up.
    //
    // mcpServer flows through the same `bindings` + `accounts` model
    // as OAuth2 connectors, but each mcpServer connector is its own
    // "account" (no cross-connector sharing): provider = "mcp",
    // account_id = connector_id, so the account_key collapses to the
    // connector id with an "mcp:" prefix.
    let now = Utc::now().to_rfc3339();
    let meta = ProviderAccount {
        provider: "mcp".to_string(),
        account_id: id.to_string(),
        email: None,
        display_name: None,
        avatar_url: None,
        granted_scopes: Vec::new(),
        connected_at: now,
        last_refresh_at: None,
        needs_reauth: false,
        // mcporter owns its own token cache on disk; we don't shadow
        // it here. These three fields stay None for "mcp" accounts.
        access_token: None,
        refresh_token: None,
        expires_at: None,
    };
    let key = account_key("mcp", id);

    let mut cfg = config.get();
    if !cfg.connectors.enabled.iter().any(|x| x == id) {
        cfg.connectors.enabled.push(id.to_string());
    }
    cfg.connectors.accounts.insert(key.clone(), meta);
    cfg.connectors.bindings.insert(id.to_string(), key);
    config.update(cfg)?;

    tracing::info!(connector_id = %id, "connector.mcp.bootstrap.ok");
    Ok(())
}

/// Revoke every OAuth cache mcporter wrote for `id`. Spawns the sidecar
/// in `--clear-mcp-oauth` mode, which uses mcporter's own primitives to
/// wipe BOTH our `tokenCacheDir` AND the shared vault at
/// `~/.mcporter/credentials.json` (see `packages/agent/src/mcp/bootstrap.ts`
/// → `clearAllOauthState`).
///
/// Why not just `std::fs::remove_dir_all(token_cache_dir)`: mcporter
/// 0.10.x writes a per-server vault entry into the shared
/// `credentials.json` file ALONGSIDE the directory. Wiping only the
/// directory leaves the vault behind, so the next install sees the
/// cached DCR client + token and "reconnects" in 2-3 seconds without
/// a real OAuth round-trip — the bug this function was rewritten to
/// fix. Always go through the sidecar so the cleanup stays consistent
/// with mcporter's own write path.
///
/// Best-effort: errors are logged, never surfaced — `disconnect` must
/// always succeed locally even if the filesystem or sidecar fights us.
pub async fn clear_oauth_caches(app: &AppHandle, id: &str) {
    let manifest = match catalog::manifest_for(id) {
        Some(m) => m,
        None => {
            tracing::warn!(connector_id = %id, "connector.mcp.clear.unknown_id");
            return;
        }
    };
    let url = match &manifest.auth {
        ConnectorAuthConfig::McpServer { transport, url }
            if matches!(
                transport,
                crate::services::connector::manifest::McpTransport::Http
            ) =>
        {
            url.clone()
        }
        _ => {
            // Not an http-MCP connector — nothing for us to clear.
            return;
        }
    };
    let dir = match token_cache_dir(app, id) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(connector_id = %id, error = %e, "connector.mcp.clear.path_failed");
            return;
        }
    };

    let payload = json!({
        "server": {
            "name": id,
            "transport": "http",
            "url": url,
            "token_cache_dir": dir.to_string_lossy(),
        },
    });

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let input_path = std::env::temp_dir().join(format!("corivo-mcp-clear-{id}-{pid}-{nanos}.json"));
    let bytes = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(connector_id = %id, error = %e, "connector.mcp.clear.serialize_failed");
            return;
        }
    };
    if let Err(e) = std::fs::write(&input_path, &bytes) {
        tracing::warn!(
            connector_id = %id,
            path = %input_path.display(),
            error = %e,
            "connector.mcp.clear.write_input_failed",
        );
        return;
    }

    let launch = match runner::locate_sidecar() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(connector_id = %id, error = %e, "connector.mcp.clear.locate_failed");
            let _ = std::fs::remove_file(&input_path);
            return;
        }
    };
    let mut cmd = launch.command_with_args([
        std::ffi::OsStr::new("--clear-mcp-oauth"),
        input_path.as_os_str(),
    ]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                connector_id = %id,
                sidecar = %launch.trace_label(),
                error = %e,
                "connector.mcp.clear.spawn_failed",
            );
            let _ = std::fs::remove_file(&input_path);
            return;
        }
    };

    let stderr = child.stderr.take();
    let stderr_task = stderr.map(|mut s| {
        let connector_id = id.to_string();
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(4 * 1024);
            let _ = s.read_to_end(&mut buf).await;
            if !buf.is_empty() {
                let text = String::from_utf8_lossy(&buf);
                for line in text.lines() {
                    if !line.is_empty() {
                        tracing::info!(
                            target: "corivo_agent",
                            connector_id = %connector_id,
                            "{line}",
                        );
                    }
                }
            }
        })
    });

    // 30s is plenty — clear is local-fs work, no network.
    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
        Ok(Ok(status)) if status.success() => {
            tracing::info!(connector_id = %id, "connector.mcp.clear.ok");
        }
        Ok(Ok(status)) => {
            tracing::warn!(connector_id = %id, %status, "connector.mcp.clear.nonzero_exit");
        }
        Ok(Err(e)) => {
            tracing::warn!(connector_id = %id, error = %e, "connector.mcp.clear.wait_failed");
        }
        Err(_) => {
            tracing::warn!(connector_id = %id, "connector.mcp.clear.timeout");
            let _ = child.kill().await;
        }
    }

    if let Some(t) = stderr_task {
        let _ = t.await;
    }
    let _ = std::fs::remove_file(&input_path);
}

/// Inputs for the runner: per-server JSON entries to drop straight into
/// `input.tools.mcp_servers`. Returns one entry per enabled
/// `mcpServer`-shape connector that has a connected account.
pub fn enabled_mcp_specs(
    app: &AppHandle,
    config: &ConfigService,
    cloud_connectors: &dyn ConnectorsService,
) -> Vec<serde_json::Value> {
    let cfg = config.get();
    let mut out = Vec::new();
    for id in &cfg.connectors.enabled {
        let manifest = match catalog::manifest_for(id) {
            Some(m) => m,
            None => continue,
        };
        let url = match &manifest.auth {
            ConnectorAuthConfig::McpServer { transport, url }
                if matches!(
                    transport,
                    crate::services::connector::manifest::McpTransport::Http
                ) =>
            {
                url
            }
            _ => continue,
        };
        // Bound (= bootstrapped) connectors only. Enabled-but-not-yet-
        // installed rows have no binding row, so they fall out here.
        if !cfg.connectors.bindings.contains_key(id) {
            continue;
        }
        let dir = match token_cache_dir(app, id) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(connector_id = %id, error = %e, "connector.mcp.specs.path_failed");
                continue;
            }
        };
        out.push(json!({
            "name": id,
            "transport": "http",
            "url": url,
            "token_cache_dir": dir.to_string_lossy(),
        }));
    }

    out.extend(cloud_connectors.hosted_mcp_specs());

    out
}
