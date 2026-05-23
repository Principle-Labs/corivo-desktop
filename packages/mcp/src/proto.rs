//! Wire types shared between the standalone `corivo-mcp` sidecar
//! binary (used by EXTERNAL MCP clients) and the `mcp_bridge` UDS
//! server in the Corivo main process.
//!
//! Two layers:
//!   1. **MCP layer** (corivo-mcp ↔ external MCP client, JSON-RPC 2.0
//!      over stdio) — we only model the minimal subset we need:
//!      `initialize`, `notifications/initialized`, `tools/list`,
//!      `tools/call`.
//!   2. **Bridge layer** (sidecar ↔ main process, JSON-line over UDS) —
//!      a thin request/response envelope. Each request carries an `id`,
//!      a `method`, and `params`; the response is `id` + `result`/`error`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Bridge layer (sidecar ⇄ main process over UDS)
// ---------------------------------------------------------------------------

/// Path of the UDS bridge socket, exposed via env var so external
/// `corivo-mcp` instances can find the main process without
/// command-line arguments (which would be visible to `ps` and harder
/// for the host MCP client's spawn flow to thread through).
pub const BRIDGE_SOCKET_ENV: &str = "CORIVO_MCP_BRIDGE_SOCK";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP layer (corivo-mcp ⇄ external MCP client over stdio JSON-RPC 2.0)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Notifications omit `id`; requests have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool definitions exposed via `tools/list`
// ---------------------------------------------------------------------------

pub const TOOL_ASK_PERMISSION: &str = "ask_permission";
pub const TOOL_RECALL_SCREEN_HISTORY: &str = "recall_screen_history";

/// Stable list of tools the sidecar exposes. Kept here so both the
/// sidecar (which advertises them) and the bridge (which dispatches
/// them) agree on names.
pub fn tool_descriptors() -> Value {
    serde_json::json!([
        {
            "name": TOOL_ASK_PERMISSION,
            "description":
                "Required permission gate. The MCP client calls this BEFORE every \
                 tool use; returns {behavior:'allow'} or {behavior:'deny',message:'…'} \
                 after the user replies in the Corivo UI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Short identifier for the action being requested."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the user should grant this permission."
                    },
                    "details": {
                        "type": "object",
                        "description": "Action-specific extra fields (e.g. command line, file path).",
                        "additionalProperties": true
                    }
                },
                "required": ["action", "reason"]
            }
        },
        {
            "name": TOOL_RECALL_SCREEN_HISTORY,
            "description":
                "Search the user's captured screen history (frames). \
                 Use this when the user references something they saw on screen \
                 (a doc, a chat, an email) and you need to locate the source frame.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Free-text search query" },
                    "limit": { "type": "integer", "default": 10 }
                },
                "required": ["query"]
            }
        }
    ])
}
