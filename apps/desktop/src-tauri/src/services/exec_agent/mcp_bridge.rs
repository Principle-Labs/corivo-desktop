//! UDS server that sits in the Corivo main process and answers requests
//! from EXTERNAL `corivo-mcp` sidecar binaries — i.e., MCP clients
//! (other apps, the user's own claude install, an `mcp-cli` invocation,
//! etc.) that the user has wired against `corivo-mcp` as their MCP
//! server.
//!
//! The desktop's own corivo-agent path no longer goes through this
//! bridge — it talks to its native AgentTool callbacks via a per-turn
//! [`super::rpc_server`] socket. The two share a `pending` map so
//! `exec_agent_permission_reply` resolves permission requests from
//! either source through the same Tauri command.
//!
//! Lifecycle:
//!   1. `McpBridge::start` binds a Unix socket under `$TMPDIR` and
//!      spawns an accept loop. The path is exposed via `socket_path()`
//!      so external `corivo-mcp` instances can dial in via the
//!      `CORIVO_MCP_BRIDGE_SOCK` env var.
//!   2. Each accepted connection runs in its own task.
//!   3. Per-line JSON requests come in (`BridgeRequest`); we dispatch on
//!      `method` and reply with `BridgeResponse`.
//!
//! Permission flow: when `ask_permission` arrives we register a oneshot
//! sender keyed by request id, emit an `exec-agent:permission-request`
//! event to the frontend, and `await` the user's reply via the oneshot.
//! The Tauri command `exec_agent_permission_reply` resolves it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
#[cfg(unix)]
use tauri::Manager;
use tauri::{AppHandle, Emitter, Runtime};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, Mutex};

#[cfg(unix)]
use crate::commands::config::AppState;
use crate::db::repos::frames::{FrameRepo, ListFramesOptions};
use crate::error::{CorivoError, Result};
use crate::services::privacy_filter::PrivacyFilter;

use corivo_mcp::proto::BRIDGE_SOCKET_ENV;
#[cfg(unix)]
use corivo_mcp::proto::{
    BridgeRequest, BridgeResponse, TOOL_ASK_PERMISSION, TOOL_RECALL_SCREEN_HISTORY,
};

/// Pending permission requests waiting for the frontend to reply.
///
/// Lives on `AppState` (not on `McpBridge`) so the per-turn rpc_server +
/// the `exec_agent_permission_reply` Tauri command can share the same
/// resolution channel even on platforms where the UDS bridge for
/// external corivo-mcp clients isn't running (e.g. Windows — see
/// `McpBridge::start`'s cfg(not(unix)) stub).
pub type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<PermissionReply>>>>;

/// Create a fresh empty pending map. AppState builds one at boot and
/// hands clones to every consumer.
pub fn new_pending_map() -> PendingMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Resolve a pending permission request. Free function so the
/// `exec_agent_permission_reply` Tauri command works whether or not
/// the long-lived UDS bridge happens to be up.
pub async fn resolve_permission(
    pending: &PendingMap,
    request_id: &str,
    reply: PermissionReply,
) -> bool {
    let mut guard = pending.lock().await;
    if let Some(tx) = guard.remove(request_id) {
        let _ = tx.send(reply);
        true
    } else {
        false
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PermissionReply {
    pub allow: bool,
    #[serde(default)]
    pub message: Option<String>,
}

pub struct McpBridge {
    socket_path: PathBuf,
    pending: PendingMap,
}

impl McpBridge {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn env_var() -> &'static str {
        BRIDGE_SOCKET_ENV
    }

    /// Shared `pending` map. Cloned from `AppState.permission_pending` at
    /// bridge start; kept on the bridge struct so the UDS accept-loop's
    /// `handle_ask_permission` can reach it without going through Tauri
    /// `State`.
    pub fn pending(&self) -> PendingMap {
        self.pending.clone()
    }

    /// Bind UDS + spawn accept loop. Returns a handle the caller stores
    /// on AppState. The accept task runs until the process exits — there
    /// is no explicit shutdown for the PoC.
    ///
    /// **Runtime context note**: tokio's `UnixListener::bind` registers
    /// the listener with the current runtime's I/O reactor — it panics
    /// when called from outside a tokio context. Tauri's `setup` closure
    /// runs on the main thread before the runtime is entered, so we
    /// `block_on` an empty async to enter the runtime for the bind call.
    #[cfg(unix)]
    pub fn start<R: Runtime>(app: AppHandle<R>, pending: PendingMap) -> Result<Arc<Self>> {
        let socket_path = make_socket_path()?;
        // Best-effort cleanup of a stale socket from a prior crash.
        let _ = std::fs::remove_file(&socket_path);
        let bind_path = socket_path.clone();
        let listener = tauri::async_runtime::block_on(
            async move { UnixListener::bind(&bind_path) },
        )
        .map_err(|e| {
            CorivoError::Internal(format!(
                "exec_agent.mcp_bridge.bind failed ({}): {e}",
                socket_path.display()
            ))
        })?;

        let bridge = Arc::new(Self {
            socket_path: socket_path.clone(),
            pending,
        });

        {
            let bridge = bridge.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((stream, _)) => {
                            tracing::info!("mcp_bridge.connection_accepted");
                            let bridge = bridge.clone();
                            let app = app.clone();
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, app, bridge).await {
                                    tracing::warn!(?error, "mcp_bridge.connection_error");
                                } else {
                                    tracing::info!("mcp_bridge.connection_closed");
                                }
                            });
                        }
                        Err(error) => {
                            tracing::error!(?error, "mcp_bridge.accept_error");
                            break;
                        }
                    }
                }
            });
        }

        tracing::info!(socket = %socket_path.display(), "mcp_bridge.started");
        Ok(bridge)
    }

    // Windows has no AF_UNIX in tokio (only blocking std::os::windows pipes).
    // External corivo-mcp clients aren't supported on Windows yet, so return
    // an error and let lib.rs's `exec_agent.bridge_start_failed` log path
    // drive the degraded experience.
    #[cfg(not(unix))]
    pub fn start<R: Runtime>(_app: AppHandle<R>, _pending: PendingMap) -> Result<Arc<Self>> {
        Err(CorivoError::Internal(
            "exec_agent.mcp_bridge: Unix-domain sockets unavailable on this platform".to_string(),
        ))
    }
}

#[cfg(unix)]
fn make_socket_path() -> Result<PathBuf> {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    dir.push(format!("corivo-mcp-{pid}.sock"));
    Ok(dir)
}

#[cfg(unix)]
async fn handle_connection<R: Runtime>(
    stream: UnixStream,
    app: AppHandle<R>,
    bridge: Arc<McpBridge>,
) -> Result<()> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    let writer = Arc::new(Mutex::new(write_half));

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: BridgeRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, line = %trimmed, "mcp_bridge.parse_failed");
                continue;
            }
        };

        let app = app.clone();
        let bridge = bridge.clone();
        let writer = writer.clone();
        // One request per task so a slow handler doesn't block the rest.
        tokio::spawn(async move {
            let response = dispatch(req, app, bridge).await;
            let mut payload = match serde_json::to_vec(&response) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(?e, "mcp_bridge.serialize_response_failed");
                    return;
                }
            };
            payload.push(b'\n');
            let mut w = writer.lock().await;
            if let Err(e) = w.write_all(&payload).await {
                tracing::warn!(?e, "mcp_bridge.write_failed");
            }
        });
    }
    Ok(())
}

#[cfg(unix)]
async fn dispatch<R: Runtime>(
    req: BridgeRequest,
    app: AppHandle<R>,
    bridge: Arc<McpBridge>,
) -> BridgeResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        TOOL_ASK_PERMISSION => match handle_ask_permission(&id, req.params, app, bridge).await {
            Ok(v) => BridgeResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => BridgeResponse {
                id,
                result: None,
                error: Some(e.to_string()),
            },
        },
        TOOL_RECALL_SCREEN_HISTORY => match handle_recall(req.params, app).await {
            Ok(v) => BridgeResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => BridgeResponse {
                id,
                result: None,
                error: Some(e.to_string()),
            },
        },
        other => BridgeResponse {
            id,
            result: None,
            error: Some(format!("unknown bridge method: {other}")),
        },
    }
}

#[cfg(unix)]
async fn handle_ask_permission<R: Runtime>(
    request_id: &str,
    params: Value,
    app: AppHandle<R>,
    bridge: Arc<McpBridge>,
) -> Result<Value> {
    // External corivo-mcp clients speak the MCP `ask_permission` schema
    // (`{ tool_name, input }`). Normalize into the unified prompt shape
    // so the dialog only has to know about one payload.
    let prompt = PermissionPrompt {
        action: params
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("(unknown tool)")
            .to_string(),
        reason: None,
        details: params.get("input").cloned(),
    };
    // External corivo-mcp clients have no Quick Ask context; their
    // permission prompts always surface in the main window.
    ask_permission_handler(request_id, prompt, app, bridge.pending.clone(), "main").await
}

#[cfg(unix)]
async fn handle_recall<R: Runtime>(params: Value, app: AppHandle<R>) -> Result<Value> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| CorivoError::Internal("AppState unavailable".to_string()))?;
    let frames_repo = state
        .frames_repo
        .as_ref()
        .ok_or_else(|| CorivoError::Internal("frames repo not initialized".to_string()))?
        .clone();
    // External corivo-mcp clients (Claude Desktop / Cursor pulling frames
    // via MCP) get the same egress redact as the in-app agent path — the
    // user's privacy toggles are the source of truth regardless of which
    // surface issued the recall.
    let privacy_filter = state.privacy_filter.clone();
    drop(state);
    recall_screen_history_handler(params, frames_repo.as_ref(), privacy_filter.as_ref()).await
}

/// Normalized prompt that both upstream callers (external MCP clients
/// and the corivo-agent native tool) shape their request into before
/// hitting [`ask_permission_handler`]. Keeps the dialog payload — and
/// therefore the React side — schema-stable regardless of which surface
/// issued the request.
///
/// - `action`: short human-readable label ("run shell command",
///   `Bash`, …). Always present; this is what the dialog headlines.
/// - `reason`: optional explanation the agent wrote for *why* it needs
///   approval. External MCP clients don't provide one.
/// - `details`: optional structured args (the original tool input,
///   action-specific fields). Rendered as JSON in the expand-on-demand
///   block.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PermissionPrompt {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Shared "ask the user for permission" plumbing. The MCP bridge
/// (external corivo-mcp clients) and the per-turn `rpc_server`
/// (corivo-agent path) both route through here so a single
/// `exec_agent_permission_reply` command resolves any caller's pending
/// request via `pending`.
pub async fn ask_permission_handler<R: Runtime>(
    request_id: &str,
    prompt: PermissionPrompt,
    app: AppHandle<R>,
    pending: PendingMap,
    target_label: &str,
) -> Result<Value> {
    let (tx, rx) = oneshot::channel::<PermissionReply>();
    {
        let mut guard = pending.lock().await;
        guard.insert(request_id.to_string(), tx);
    }

    let payload = json!({
        "id": request_id,
        "action": prompt.action,
        "reason": prompt.reason,
        "details": prompt.details,
    });
    // Route to the originating window only (main vs. quick-ask) so the
    // user sees the prompt where the request actually came from.
    if let Err(e) = app.emit_to(target_label, "exec-agent:permission-request", payload) {
        // No listener / frontend gone — auto-deny so the agent doesn't hang.
        let mut guard = pending.lock().await;
        guard.remove(request_id);
        return Ok(json!({ "behavior": "deny", "message": format!("emit failed: {e}") }));
    }

    let reply = rx.await.unwrap_or(PermissionReply {
        allow: false,
        message: Some("permission channel dropped".to_string()),
    });

    Ok(json!({
        "behavior": if reply.allow { "allow" } else { "deny" },
        "message": reply.message,
    }))
}

/// Shared FTS recall handler. Both the external-client MCP bridge and
/// the per-turn `rpc_server` (corivo-agent path) call into this so the
/// wire shape stays a single source of truth — the corivo-agent native
/// tool's `result.frames` shape lines up with what `corivo-mcp`
/// returns to its MCP clients.
pub async fn recall_screen_history_handler(
    params: Value,
    frames_repo: &dyn FrameRepo,
    privacy_filter: &PrivacyFilter,
) -> Result<Value> {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| CorivoError::Internal("recall: missing 'query'".to_string()))?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .min(50) as usize;

    let opts = ListFramesOptions {
        limit: limit as u32,
        ..Default::default()
    };
    let results = frames_repo
        .fts_search(query, opts)
        .await
        .map_err(|e| CorivoError::Internal(format!("recall search failed: {e}")))?;

    // Egress redact each frame's snippet before handing it to the LLM /
    // MCP client. Only the `snippet` field is filtered — `window_title`
    // and `app_bundle_id` pass through unchanged (per user's scope choice).
    // `classify_and_enforce` is a fast no-op when settings.enabled=false,
    // and a cache lookup when the same snippet was redacted earlier this
    // session; the worst case is `limit` (<=50) serial ONNX runs for an
    // entirely cold cache, which the LRU then absorbs on subsequent calls.
    let mut frames: Vec<Value> = Vec::with_capacity(results.len());
    for f in results {
        let raw_snippet = preview(f.ax_text.as_deref(), f.ocr_text.as_deref(), 240);
        let snippet = privacy_filter.classify_and_enforce(&raw_snippet).await;
        frames.push(json!({
            "id": f.id,
            "captured_at": f.captured_at,
            "app_bundle_id": f.app_bundle_id,
            "window_title": f.window_title,
            "snippet": snippet,
        }));
    }

    Ok(json!({ "frames": frames }))
}

fn preview(ax: Option<&str>, ocr: Option<&str>, max: usize) -> String {
    let primary = ax.filter(|s| !s.trim().is_empty()).or(ocr).unwrap_or("");
    let trimmed: String = primary.chars().take(max).collect();
    if primary.chars().count() > max {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}
