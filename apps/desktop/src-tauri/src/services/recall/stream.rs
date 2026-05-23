//! Stream emitter for `/ask`.
//!
//! Emits JSON-Lines events the frontend `useChatStream` hook consumes:
//!
//! ```jsonl
//! {"type":"text-delta","text":"hello"}
//! {"type":"tool-call","id":"call_1","name":"search_frames","args":{...}}
//! {"type":"tool-result","id":"call_1","result":{...}}
//! {"type":"cited-frames","frame_ids":["F1","F2"]}
//! {"type":"status","kind":"api-retry","detail":"Network retry (2/8): ECONNRESET"}
//! {"type":"finish","reason":"end-turn"}
//! {"type":"error","message":"…"}
//! ```
//!
//! Each line is one JSON object terminated by `\n`. Emitted to a Tauri
//! `Channel<String>` so it flows over IPC without an HTTP server.
//!
//! Phase 3 ships this format. Spec §八 picks Vercel AI SDK Data Stream
//! Protocol as the long-term target — Phase 3.5 will swap the line shape
//! over without changing the orchestrator (the emitter is the only place
//! that knows the wire format).

use serde::Serialize;
use serde_json::{json, Value};
use tauri::ipc::Channel;

use crate::error::{CorivoError, Result};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FinishReason {
    EndTurn,
    Truncated,
    Error,
    /// User-initiated cancellation — distinct from `Error` so the UI can
    /// render a neutral "已取消" state instead of an error chrome. Phase B
    /// addition; matches §5.2 of the agent-sidecar spec.
    Cancelled,
}

pub struct StreamEmitter {
    channel: Channel<String>,
}

impl StreamEmitter {
    pub fn new(channel: Channel<String>) -> Self {
        Self { channel }
    }

    fn send(&self, value: Value) -> Result<()> {
        let mut line = value.to_string();
        line.push('\n');
        self.channel
            .send(line)
            .map_err(|error| CorivoError::Internal(format!("chat channel send failed: {error}")))
    }

    pub fn text_delta(&self, text: &str) -> Result<()> {
        self.send(json!({"type": "text-delta", "text": text}))
    }

    pub fn tool_call(&self, id: &str, name: &str, args: &Value) -> Result<()> {
        self.send(json!({
            "type": "tool-call",
            "id": id,
            "name": name,
            "args": args,
        }))
    }

    pub fn tool_result(&self, id: &str, result: &Value) -> Result<()> {
        self.send(json!({
            "type": "tool-result",
            "id": id,
            "result": result,
        }))
    }

    pub fn cited_frames(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.send(json!({
            "type": "cited-frames",
            "frame_ids": ids,
        }))
    }

    pub fn finish(&self, reason: FinishReason) -> Result<()> {
        self.send(json!({
            "type": "finish",
            "reason": reason,
        }))
    }

    pub fn error(&self, message: &str) -> Result<()> {
        self.send(json!({
            "type": "error",
            "message": message,
        }))
    }

    /// Transient progress hint that doesn't change message content.
    /// The frontend treats `kind` as a tag (e.g. `"api-retry"`) and
    /// renders `detail` next to the thinking dots while the assistant
    /// turn is still streaming.
    pub fn status(&self, kind: &str, detail: Option<&str>) -> Result<()> {
        self.send(json!({
            "type": "status",
            "kind": kind,
            "detail": detail,
        }))
    }
}
