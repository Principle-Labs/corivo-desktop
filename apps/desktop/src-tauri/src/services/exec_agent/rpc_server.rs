//! Per-turn UDS JSON-RPC server for corivo-agent's native AgentTool
//! callbacks (spec §5.4).
//!
//! Lifecycle:
//!   - `start(socket_path, deps)` binds a `UnixListener` and spawns an
//!     accept loop on the current tokio runtime; returns a
//!     [`RpcServerHandle`] the runner uses to tear the listener down
//!     after the turn finishes.
//!   - Each accepted connection reads line-delimited JSON-RPC requests,
//!     dispatches by `method`, writes a one-line JSON response.
//!   - Method whitelist (matches §5.4):
//!       * `recall_screen_history` → shared FTS handler (also used by
//!         `mcp_bridge` for external corivo-mcp clients).
//!       * `ask_permission` → window-emit + oneshot dance, sharing the
//!         long-lived `mcp_bridge`'s `pending` map so a single
//!         `exec_agent_permission_reply` command resolves regardless of
//!         which surface issued the request.
//!   - Unknown methods return JSON-RPC `-32601` (method not found).
//!
//! Per-turn server (vs. the long-lived `mcp_bridge`) means the socket
//! path encodes pid + nanos so concurrent turns can't collide.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(any(unix, windows))]
use std::time::Instant;

#[cfg(any(unix, windows))]
use serde::{Deserialize, Serialize};
#[cfg(any(unix, windows))]
use serde_json::{json, Value};
use tauri::{AppHandle, Wry};
#[cfg(any(unix, windows))]
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::db::repos::chat::{ChatMessageRepo, ChatThreadRepo};
use crate::db::repos::frames::FrameRepo;
use crate::db::repos::notes::NotesRepo;
use crate::error::{CorivoError, Result};
use crate::services::privacy_filter::PrivacyFilter;
use crate::services::scheduled_workflows::WorkflowStore;

use super::mcp_bridge::PendingMap;
#[cfg(any(unix, windows))]
use super::mcp_bridge::{ask_permission_handler, recall_screen_history_handler, PermissionPrompt};
#[cfg(any(unix, windows))]
use super::memory_tools::{
    auto_persona_get_previous_handler, chat_thread_get_handler, memory_search_handler,
    note_list_handler, thread_search_handler,
};
#[cfg(any(unix, windows))]
use super::save_note_handler::save_note_handler;
use super::schedule_task_handler::{
    cancel_scheduled_task_handler, list_scheduled_tasks_handler, schedule_task_handler,
    update_scheduled_task_handler,
};

/// JSON-RPC method-not-found code (the only standard one we use today).
#[cfg(any(unix, windows))]
const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC "internal error" — used as a catch-all for handler failures.
#[cfg(any(unix, windows))]
const JSONRPC_INTERNAL: i32 = -32603;

/// Shared dependencies the server hands to each request handler. Built
/// fresh per turn from `AppState` so the runner doesn't have to keep
/// `&AppState` alive across the spawn boundary (which would force every
/// caller to thread a Tauri State through, defeating the point of a
/// per-turn handle).
#[derive(Clone)]
pub struct RpcDeps {
    pub app: AppHandle<Wry>,
    pub frames_repo: Arc<dyn FrameRepo>,
    /// Optional because Step 1a wires it in, but the boot order in
    /// `lib.rs` could race and leave it `None` momentarily. Handlers
    /// surface a clear error in that case.
    pub notes_repo: Option<Arc<dyn NotesRepo>>,
    pub chat_threads: Option<Arc<dyn ChatThreadRepo>>,
    pub chat_messages: Option<Arc<dyn ChatMessageRepo>>,
    /// Current thread driving the turn — used as the default
    /// `source_thread_id` when the agent calls `save_note` without an
    /// explicit thread/message anchor.
    pub thread_id: String,
    pub pending: PendingMap,
    /// Window label that should receive `exec-agent:permission-request`
    /// for this turn (e.g. `"main"` for /ask, `"quick-ask"` for QA).
    /// Routed through `emit_to` so the prompt surfaces where the user
    /// actually started the conversation.
    pub permission_target: String,
    /// memory-system-spec §4 — directory holding `auto-persona.md`,
    /// the soft-persona document the distill task writes.
    pub app_data_dir: std::path::PathBuf,
    /// v1431 — scheduled-workflows store, used by the `schedule_task`
    /// family of native tools. `None` if AppState wasn't fully wired
    /// (e.g. mid-boot race or test harness without the workflows
    /// service); handlers surface a clear error in that case.
    pub workflow_store: Option<std::sync::Arc<WorkflowStore>>,
    /// Egress PII redactor — applied to every frame `snippet` returned
    /// by `recall_screen_history` before it's handed to the LLM. Always
    /// `Some` because `PrivacyFilter` is a pure-memory object with no
    /// IPC/DB dependency (constructed at boot in `lib.rs`). When the
    /// user has settings.enabled=false this is a fast no-op.
    pub privacy_filter: Arc<PrivacyFilter>,
}

/// Handle to a running RPC server. Drop or call `shutdown()` to stop the
/// accept loop and unlink the socket.
pub struct RpcServerHandle {
    socket_path: PathBuf,
    accept_task: Mutex<Option<JoinHandle<()>>>,
}

impl RpcServerHandle {
    /// Cancel the accept task and unlink the socket file. Safe to call
    /// multiple times.
    pub async fn shutdown(self) {
        if let Some(task) = self.accept_task.lock().await.take() {
            task.abort();
            // Best-effort wait; a join error after abort is expected.
            let _ = task.await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Synchronous shutdown used from error-path code that doesn't have
    /// an `await`-able context (e.g. spawn failed before we entered the
    /// pump loop). Aborts the task without awaiting.
    pub fn shutdown_blocking(self) {
        if let Ok(mut guard) = self.accept_task.try_lock() {
            if let Some(task) = guard.take() {
                task.abort();
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Windows named-pipe equivalent of the UDS accept loop below.
///
/// Lifecycle:
///   1. Create the first `NamedPipeServer` instance bound to `socket_path`
///      (must be `\\.\pipe\<name>`). `first_pipe_instance(true)` makes
///      CreateNamedPipeW fail if another process already owns the name —
///      preferable to silently sharing it.
///   2. The accept task awaits `server.connect()` (the equivalent of
///      UnixListener::accept) and as soon as a client lands, hands the
///      connected pipe off to `handle_connection` and creates the NEXT
///      server instance for the following client. This avoids the
///      well-known race where a client could try to connect between us
///      finishing one connect() and starting the next instance.
///
/// Node's `net.connect(path)` on Windows transparently treats a
/// `\\.\pipe\...` path as a named-pipe path — the agent's rpc.ts works
/// here unchanged.
#[cfg(windows)]
pub async fn start(socket_path: &Path, deps: RpcDeps) -> Result<RpcServerHandle> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    // `ServerOptions::create` takes an OsStr / impl AsRef<OsStr>; widening
    // through OsStr keeps the API portable across drive-letter / UNC
    // shapes if the path ever gains them.
    let _: Vec<u16> = OsStr::new(socket_path).encode_wide().collect();

    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(socket_path)
        .map_err(|e| {
            CorivoError::Internal(format!(
                "exec_agent.rpc_server.create_pipe failed ({}): {e}",
                socket_path.display()
            ))
        })?;

    tracing::info!(
        socket = %socket_path.display(),
        "exec_agent.rpc_server.started"
    );

    let path_owned = socket_path.to_path_buf();
    let task = tokio::spawn(async move {
        loop {
            if let Err(error) = server.connect().await {
                tracing::warn!(?error, "exec_agent.rpc_server.connect_error");
                break;
            }
            tracing::debug!("exec_agent.rpc_server.connection_accepted");

            // Take the connected pipe; build the next server instance so
            // the next client doesn't race the gap.
            let connected = server;
            server = match ServerOptions::new().create(&path_owned) {
                Ok(s) => s,
                Err(error) => {
                    tracing::error!(?error, "exec_agent.rpc_server.recreate_pipe_failed");
                    // Still service the connected client; we just can't
                    // accept further connections on this turn.
                    let deps = deps.clone();
                    tokio::spawn(async move {
                        let (read_half, write_half) = tokio::io::split(connected);
                        if let Err(error) =
                            handle_connection_io(read_half, write_half, deps).await
                        {
                            tracing::warn!(?error, "exec_agent.rpc_server.connection_error");
                        }
                    });
                    break;
                }
            };

            let deps = deps.clone();
            tokio::spawn(async move {
                let (read_half, write_half) = tokio::io::split(connected);
                if let Err(error) = handle_connection_io(read_half, write_half, deps).await {
                    tracing::warn!(?error, "exec_agent.rpc_server.connection_error");
                }
            });
        }
    });

    Ok(RpcServerHandle {
        socket_path: socket_path.to_path_buf(),
        accept_task: Mutex::new(Some(task)),
    })
}

/// Bind the UDS socket and spawn the accept loop on the current
/// tokio runtime. Returns once the listener is ready — callers can spawn
/// the sidecar immediately after without racing the bind.
#[cfg(unix)]
pub async fn start(socket_path: &Path, deps: RpcDeps) -> Result<RpcServerHandle> {
    let listener = UnixListener::bind(socket_path).map_err(|e| {
        CorivoError::Internal(format!(
            "exec_agent.rpc_server.bind failed ({}): {e}",
            socket_path.display()
        ))
    })?;

    tracing::info!(
        socket = %socket_path.display(),
        "exec_agent.rpc_server.started"
    );

    let task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tracing::debug!("exec_agent.rpc_server.connection_accepted");
                    let deps = deps.clone();
                    tokio::spawn(async move {
                        let (read_half, write_half) = stream.into_split();
                        if let Err(error) =
                            handle_connection_io(read_half, write_half, deps).await
                        {
                            tracing::warn!(?error, "exec_agent.rpc_server.connection_error");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(?error, "exec_agent.rpc_server.accept_error");
                    break;
                }
            }
        }
    });

    Ok(RpcServerHandle {
        socket_path: socket_path.to_path_buf(),
        accept_task: Mutex::new(Some(task)),
    })
}

#[cfg(any(unix, windows))]
#[derive(Debug, Clone, Deserialize)]
struct RpcRequest {
    /// Optional in JSON-RPC 2.0 (notifications omit it). We never emit
    /// notifications back to the sidecar, so we tolerate `null`.
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[cfg(any(unix, windows))]
#[derive(Debug, Clone, Serialize)]
struct RpcResponse {
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[cfg(any(unix, windows))]
#[derive(Debug, Clone, Serialize)]
struct RpcError {
    code: i32,
    message: String,
    /// Short snake_case tag the sidecar can branch on (e.g.
    /// `recall_failed`). JSON-RPC `data` field, kept generic.
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// Shared connection handler driven by an already-split (reader, writer)
/// pair. Reading and writing must be independent — both `UnixStream::into_split`
/// (unix) and `tokio::io::split` (windows named pipes) satisfy that. The
/// earlier Windows-only variant put the whole `NamedPipeServer` behind a
/// single `Mutex`, which deadlocked the moment a dispatch task tried to
/// write a response back while the read loop was parked on the next-line
/// `await` — that path is gone now.
#[cfg(any(unix, windows))]
async fn handle_connection_io<R, W>(reader: R, writer: W, deps: RpcDeps) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, line = %trimmed, "exec_agent.rpc_server.parse_failed");
                continue;
            }
        };

        let deps = deps.clone();
        let writer = writer.clone();
        let method = req.method.clone();
        let request_started_at = Instant::now();
        // One request per task so a slow handler doesn't block the rest.
        tokio::spawn(async move {
            let response = dispatch(req, deps).await;
            tracing::info!(
                target: "exec_agent",
                method = %method,
                success = response.error.is_none(),
                phase_ms = elapsed_ms(request_started_at),
                "exec_agent.rpc_server.request_done"
            );
            let mut payload = match serde_json::to_vec(&response) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(?e, "exec_agent.rpc_server.serialize_response_failed");
                    return;
                }
            };
            payload.push(b'\n');
            let mut w = writer.lock().await;
            if let Err(e) = w.write_all(&payload).await {
                tracing::warn!(?e, "exec_agent.rpc_server.write_failed");
            }
        });
    }
    Ok(())
}

#[cfg(any(unix, windows))]
async fn dispatch(req: RpcRequest, deps: RpcDeps) -> RpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "recall_screen_history" => {
            match recall_screen_history_handler(
                req.params,
                deps.frames_repo.as_ref(),
                deps.privacy_filter.as_ref(),
            )
            .await
            {
                Ok(v) => RpcResponse {
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: JSONRPC_INTERNAL,
                        message: e.to_string(),
                        data: Some(json!({ "code": "recall_failed" })),
                    }),
                },
            }
        }
        "chat_thread_get" => match chat_thread_get_handler(
            req.params,
            deps.chat_threads.as_deref(),
            deps.chat_messages.as_deref(),
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "chat_thread_get_failed" })),
                }),
            },
        },
        "note_list" => match note_list_handler(req.params, deps.notes_repo.as_deref()).await {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "note_list_failed" })),
                }),
            },
        },
        "auto_persona_get_previous" => {
            match auto_persona_get_previous_handler(req.params, &deps.app_data_dir).await {
                Ok(v) => RpcResponse {
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: JSONRPC_INTERNAL,
                        message: e.to_string(),
                        data: Some(json!({ "code": "auto_persona_get_previous_failed" })),
                    }),
                },
            }
        }
        "memory_search" => match memory_search_handler(
            req.params,
            Some(deps.frames_repo.clone()),
            deps.notes_repo.clone(),
            deps.chat_messages.clone(),
            &deps.thread_id,
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "memory_search_failed" })),
                }),
            },
        },
        "thread_search" => {
            match thread_search_handler(req.params, deps.chat_threads.as_deref()).await {
                Ok(v) => RpcResponse {
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: JSONRPC_INTERNAL,
                        message: e.to_string(),
                        data: Some(json!({ "code": "thread_search_failed" })),
                    }),
                },
            }
        }
        "save_note" => {
            match save_note_handler(req.params, deps.notes_repo.as_deref(), &deps.thread_id).await {
                Ok(v) => RpcResponse {
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: JSONRPC_INTERNAL,
                        message: e.to_string(),
                        data: Some(json!({ "code": "save_note_failed" })),
                    }),
                },
            }
        }
        "schedule_task" => match schedule_task_handler(
            req.params,
            deps.workflow_store.as_ref(),
            &deps.thread_id,
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "schedule_task_failed" })),
                }),
            },
        },
        "list_scheduled_tasks" => match list_scheduled_tasks_handler(
            req.params,
            deps.workflow_store.as_ref(),
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "list_scheduled_tasks_failed" })),
                }),
            },
        },
        "cancel_scheduled_task" => match cancel_scheduled_task_handler(
            req.params,
            deps.workflow_store.as_ref(),
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "cancel_scheduled_task_failed" })),
                }),
            },
        },
        "update_scheduled_task" => match update_scheduled_task_handler(
            req.params,
            deps.workflow_store.as_ref(),
        )
        .await
        {
            Ok(v) => RpcResponse {
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => RpcResponse {
                id,
                result: None,
                error: Some(RpcError {
                    code: JSONRPC_INTERNAL,
                    message: e.to_string(),
                    data: Some(json!({ "code": "update_scheduled_task_failed" })),
                }),
            },
        },
        "ask_permission" => {
            // The sidecar generates rpc-ids itself; we use those as the
            // pending-key so Phase B can reuse the existing
            // `exec_agent_permission_reply` Tauri command unchanged.
            let request_id = match req.id.as_str() {
                Some(s) => s.to_string(),
                None => req.id.to_string(),
            };
            // corivo-agent's native ask_permission tool sends
            // `{ action, reason, details }` (see
            // packages/agent/src/native-tools/ask-permission.ts).
            // Normalize into the shared prompt shape so the dialog
            // payload stays uniform across MCP and agent callers.
            let prompt = PermissionPrompt {
                action: req
                    .params
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("(unspecified action)")
                    .to_string(),
                reason: req
                    .params
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                details: req.params.get("details").cloned(),
            };
            match ask_permission_handler(
                &request_id,
                prompt,
                deps.app,
                deps.pending,
                &deps.permission_target,
            )
            .await
            {
                Ok(v) => RpcResponse {
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => RpcResponse {
                    id,
                    result: None,
                    error: Some(RpcError {
                        code: JSONRPC_INTERNAL,
                        message: e.to_string(),
                        data: Some(json!({ "code": "ask_permission_failed" })),
                    }),
                },
            }
        }
        other => RpcResponse {
            id,
            result: None,
            error: Some(RpcError {
                code: JSONRPC_METHOD_NOT_FOUND,
                message: format!("unknown method: {other}"),
                data: Some(json!({ "code": "method_not_found" })),
            }),
        },
    }
}

#[cfg(any(unix, windows))]
fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(all(any(unix, windows), test))]
mod tests {
    use super::*;

    #[test]
    fn rpc_response_ok_serializes_without_error_field() {
        let resp = RpcResponse {
            id: json!("req-1"),
            result: Some(json!({"ok": true})),
            error: None,
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"id\":\"req-1\""));
        assert!(s.contains("\"result\""));
        assert!(!s.contains("\"error\""));
    }

    #[test]
    fn rpc_response_err_serializes_with_data_tag() {
        let resp = RpcResponse {
            id: json!("req-2"),
            result: None,
            error: Some(RpcError {
                code: JSONRPC_METHOD_NOT_FOUND,
                message: "unknown method: foo".into(),
                data: Some(json!({"code": "method_not_found"})),
            }),
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("-32601"));
        assert!(s.contains("method_not_found"));
        assert!(!s.contains("\"result\""));
    }
}
