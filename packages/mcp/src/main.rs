//! `corivo-mcp` — sidecar MCP server spawned by claude CLI as a child
//! process via `--mcp-config`. Talks JSON-RPC 2.0 over stdio (the MCP
//! "stdio transport") on the claude side, and forwards every tool call
//! to the Corivo main process via a UDS bridge.
//!
//! Wire format (stdio): one JSON-RPC message per line, framed by `\n`.
//!
//! Subset of MCP we implement:
//!   - `initialize` (request) → return server info + tool capability
//!   - `notifications/initialized` (notification) → no-op
//!   - `tools/list` (request) → return our two tools
//!   - `tools/call` (request) → forward to bridge, wrap result as MCP content
//!
//! Anything else returns method-not-found (-32601).

use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use corivo_mcp::proto::{
    tool_descriptors, BridgeRequest, BridgeResponse, JsonRpcRequest, JsonRpcResponse,
    BRIDGE_SOCKET_ENV, TOOL_ASK_PERMISSION, TOOL_RECALL_SCREEN_HISTORY,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::{oneshot, Mutex};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<BridgeResponse>>>>;

/// Append-only sidecar log under `$TMPDIR/corivo-mcp-sidecar-{pid}.log`.
/// Claude swallows our stderr in some setups, so the log file is the
/// reliable channel for "did the sidecar receive the initialize request,
/// did it respond, did claude hand it tools/call …".
static LOG_FILE: OnceLock<StdMutex<Option<std::fs::File>>> = OnceLock::new();

fn log_init() -> std::path::PathBuf {
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("corivo-mcp-sidecar-{pid}.log"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    let _ = LOG_FILE.set(StdMutex::new(file));
    path
}

fn log(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("[{stamp}] {msg}");
    eprintln!("{line}");
    if let Some(lock) = LOG_FILE.get() {
        if let Ok(mut guard) = lock.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{line}");
                let _ = f.flush();
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let log_path = log_init();
    log(format!(
        "starting (pid={}, version={}, log={})",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        log_path.display()
    ));

    let socket_path = std::env::var(BRIDGE_SOCKET_ENV).unwrap_or_default();
    log(format!("env {BRIDGE_SOCKET_ENV}={socket_path:?}"));
    if socket_path.is_empty() {
        log("WARNING bridge socket env not set — tools/call will be degraded");
    }

    let bridge: Option<Arc<BridgeClient>> = if socket_path.is_empty() {
        None
    } else {
        log(format!("attempting bridge connect to {socket_path}"));
        match BridgeClient::connect(&socket_path).await {
            Ok(c) => {
                log(format!("bridge connected ({socket_path})"));
                Some(Arc::new(c))
            }
            Err(e) => {
                log(format!(
                    "bridge connect failed ({socket_path}): {e} — degraded mode"
                ));
                None
            }
        }
    };

    log("entering stdin loop");
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                log(format!("parse failed: {e} (line: {trimmed})"));
                continue;
            }
        };
        log(format!("recv method={} id={:?}", req.method, req.id));

        let method = req.method.clone();
        let response = handle(req, bridge.clone()).await;
        match &response {
            Some(r) => log(format!(
                "send id={} result_some={} error_some={}",
                r.id,
                r.result.is_some(),
                r.error.is_some()
            )),
            None => log(format!("notification {method} → no response")),
        }
        if let Some(resp) = response {
            let mut payload = match serde_json::to_vec(&resp) {
                Ok(b) => b,
                Err(e) => {
                    log(format!("serialize failed: {e}"));
                    continue;
                }
            };
            payload.push(b'\n');
            if let Err(e) = stdout.write_all(&payload).await {
                log(format!("stdout write failed: {e}"));
                return ExitCode::from(1);
            }
            let _ = stdout.flush().await;
        }
    }

    log("stdin EOF — exiting");
    ExitCode::SUCCESS
}

async fn handle(
    req: JsonRpcRequest,
    bridge: Option<Arc<BridgeClient>>,
) -> Option<JsonRpcResponse> {
    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::ok(
            id.unwrap_or(Value::Null),
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "Corivo", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "notifications/initialized" | "notifications/cancelled" => None,
        "tools/list" => Some(JsonRpcResponse::ok(
            id.unwrap_or(Value::Null),
            json!({ "tools": tool_descriptors() }),
        )),
        "tools/call" => {
            let id = id.unwrap_or(Value::Null);
            let name = req
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            if name != TOOL_ASK_PERMISSION && name != TOOL_RECALL_SCREEN_HISTORY {
                return Some(JsonRpcResponse::err(id, -32601, format!("unknown tool: {name}")));
            }

            let Some(bridge) = bridge else {
                return Some(JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": "Corivo bridge unavailable — sidecar started without a UDS \
                                     connection. Check that the main Corivo process is running \
                                     and CORIVO_MCP_BRIDGE_SOCK points at its bound socket.",
                        }],
                        "isError": true,
                    }),
                ));
            };

            match bridge.call(&name, arguments).await {
                Ok(value) => Some(JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": value.to_string(),
                        }],
                        "isError": false,
                    }),
                )),
                Err(e) => Some(JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": format!("bridge error: {e}"),
                        }],
                        "isError": true,
                    }),
                )),
            }
        }
        _ => Some(JsonRpcResponse::err(
            id.unwrap_or(Value::Null),
            -32601,
            format!("method not found: {}", req.method),
        )),
    }
}

// ---------------------------------------------------------------------------
// Bridge client (UDS, line-delimited JSON request/response)
// ---------------------------------------------------------------------------

struct BridgeClient {
    writer: Mutex<OwnedWriteHalf>,
    next_id: Mutex<u64>,
    pending: Pending,
}

impl BridgeClient {
    async fn connect(path: &str) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (read_half, write_half) = stream.into_split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(read_loop(read_half, pending.clone()));
        Ok(Self {
            writer: Mutex::new(write_half),
            next_id: Mutex::new(0),
            pending,
        })
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut n = self.next_id.lock().await;
            *n += 1;
            format!("req-{n}")
        };
        self.pending.lock().await.insert(id.clone(), tx);

        let req = BridgeRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let mut payload = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        payload.push(b'\n');
        {
            let mut w = self.writer.lock().await;
            w.write_all(&payload).await.map_err(|e| e.to_string())?;
        }

        let resp = rx.await.map_err(|_| "bridge channel dropped".to_string())?;
        if let Some(err) = resp.error {
            Err(err)
        } else {
            Ok(resp.result.unwrap_or(Value::Null))
        }
    }
}

async fn read_loop(read_half: OwnedReadHalf, pending: Pending) {
    let mut lines = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resp: BridgeResponse = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("corivo-mcp: bridge parse failed: {e}");
                continue;
            }
        };
        let mut guard = pending.lock().await;
        if let Some(tx) = guard.remove(&resp.id) {
            let _ = tx.send(resp);
        }
    }
}
