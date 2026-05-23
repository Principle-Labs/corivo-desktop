//! Parse the corivo-agent sidecar's NDJSON event stream (spec §5.2) and
//! forward each event into the existing `StreamEmitter` wire shape so the
//! frontend `useChatStream` hook stays untouched.
//!
//! Wire envelope:
//!
//! ```jsonl
//! {"v":1,"ts":1700000000000,"type":"text_delta","data":{"delta":"hi"}}
//! ```
//!
//! Producer of this schema lives in `packages/agent/src/events.ts`; the
//! two MUST stay in sync. When in doubt, prefer changing the Rust side
//! to match the TS producer (TS already ran a Phase A smoke test).

use serde::Deserialize;
use serde_json::Value;

use crate::error::{CorivoError, Result};
use crate::services::recall::stream::{FinishReason, StreamEmitter};

/// Top-level envelope every corivo-agent event is wrapped in.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    #[allow(dead_code)]
    pub v: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub ts: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
}

/// Mirror of `FinishReason` strings produced by the sidecar (`events.ts`).
fn map_finish_reason(raw: &str) -> FinishReason {
    match raw {
        "EndTurn" | "ToolUse" => FinishReason::EndTurn,
        "Truncated" => FinishReason::Truncated,
        "Cancelled" => FinishReason::Cancelled,
        "Error" | "Refusal" => FinishReason::Error,
        // Forward-compat: unknown reasons fall through to EndTurn rather
        // than Error so a future sidecar variant doesn't surface as a
        // user-visible failure for what was a clean turn.
        _ => FinishReason::EndTurn,
    }
}

pub fn parse_line(line: &str) -> Result<Option<Envelope>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let env: Envelope = serde_json::from_str(trimmed).map_err(|e| {
        CorivoError::Internal(format!(
            "exec_agent.corivo.parse_line failed: {e} (line: {trimmed})"
        ))
    })?;
    Ok(Some(env))
}

/// Forward one event to the emitter. Returns `Some(reason)` exactly when
/// the event signals end-of-turn (a `agent_end` line); the caller should
/// then stop pumping.
pub fn forward(envelope: Envelope, emitter: &StreamEmitter) -> Result<Option<FinishReason>> {
    match envelope.event_type.as_str() {
        "agent_start" | "turn_start" => Ok(None),
        "text_delta" => {
            if let Some(delta) = envelope.data.get("delta").and_then(Value::as_str) {
                emitter.text_delta(delta)?;
            }
            Ok(None)
        }
        // We don't surface thinking deltas to the chat transcript today.
        // Phase B keeps them silent; a future UI can opt in.
        "thinking_delta" => Ok(None),
        // `tool_call_end` carries the final argument shape; that's when
        // the model has decided what to call. `tool_call_start` is the
        // partial-arguments stream which we ignore for now (the Claude
        // path also emits a single tool-call event).
        "tool_call_end" => {
            let id = envelope
                .data
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = envelope
                .data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            // `arguments` arrives as a JSON-encoded string; try to parse
            // back into a Value for the emitter, but fall back to the
            // raw string if it's not valid JSON (defensive — should
            // never trigger with the spec'd producer).
            let args = envelope
                .data
                .get("arguments")
                .and_then(Value::as_str)
                .map(|s| {
                    serde_json::from_str::<Value>(s)
                        .unwrap_or_else(|_| Value::String(s.to_string()))
                })
                .unwrap_or(Value::Null);
            emitter.tool_call(id, name, &args)?;
            Ok(None)
        }
        "tool_call_start" | "tool_execution_start" | "tool_execution_update" => Ok(None),
        "tool_execution_end" => {
            let id = envelope
                .data
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let result = envelope.data.get("result").cloned().unwrap_or(Value::Null);
            emitter.tool_result(id, &result)?;
            Ok(None)
        }
        "cited_frames" => {
            if let Some(arr) = envelope.data.get("frame_ids").and_then(Value::as_array) {
                let ids: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                emitter.cited_frames(&ids)?;
            }
            Ok(None)
        }
        // Phase B no-ops: carry through but don't surface to the UI yet.
        "compaction_start" | "compaction_end" | "turn_end" => Ok(None),
        "api_retry" => {
            let attempt = envelope
                .data
                .get("attempt")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let max_attempts = envelope
                .data
                .get("max_attempts")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let detail = format_retry_detail(attempt, max_attempts);
            emitter.status("api-retry", Some(&detail))?;
            Ok(None)
        }
        "agent_end" => {
            let reason = envelope
                .data
                .get("finish_reason")
                .and_then(Value::as_str)
                .map(map_finish_reason)
                .unwrap_or(FinishReason::EndTurn);
            Ok(Some(reason))
        }
        "error" => {
            let msg = envelope
                .data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("corivo-agent reported an unspecified error");
            emitter.error(msg)?;
            // The sidecar will follow with an `agent_end` envelope; let
            // that drive the finish state so the caller sees one terminal
            // event. But if the sidecar dies before emitting agent_end,
            // the runner's wait-on-EOF path turns the orphan into Error.
            Ok(None)
        }
        // Forward-compat: ignore unknown variants rather than aborting
        // the whole turn — the schema isn't versioned beyond `v` and new
        // event types may ship at any release.
        _ => {
            tracing::debug!(
                event_type = %envelope.event_type,
                "exec_agent.corivo.unknown_event_skipped"
            );
            Ok(None)
        }
    }
}

fn format_retry_detail(attempt: Option<u32>, max_attempts: Option<u32>) -> String {
    match (attempt, max_attempts) {
        (Some(n), Some(max)) => format!("Network retry ({n}/{max})…"),
        (Some(n), None) => format!("Network retry (#{n})…"),
        _ => "Network retry…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_finish_reasons() {
        assert!(matches!(
            map_finish_reason("EndTurn"),
            FinishReason::EndTurn
        ));
        assert!(matches!(
            map_finish_reason("ToolUse"),
            FinishReason::EndTurn
        ));
        assert!(matches!(
            map_finish_reason("Cancelled"),
            FinishReason::Cancelled
        ));
        assert!(matches!(map_finish_reason("Error"), FinishReason::Error));
        assert!(matches!(map_finish_reason("Refusal"), FinishReason::Error));
        assert!(matches!(
            map_finish_reason("Truncated"),
            FinishReason::Truncated
        ));
        // Unknown variants fall through to EndTurn (forward-compat).
        assert!(matches!(
            map_finish_reason("Mystery"),
            FinishReason::EndTurn
        ));
    }

    #[test]
    fn parses_envelope_and_extracts_text_delta() {
        let line = r#"{"v":1,"ts":1700000000000,"type":"text_delta","data":{"delta":"hi"}}"#;
        let env = parse_line(line).unwrap().unwrap();
        assert_eq!(env.event_type, "text_delta");
        assert_eq!(env.data.get("delta").and_then(Value::as_str), Some("hi"));
    }

    #[test]
    fn parse_skips_blank_lines() {
        assert!(parse_line("").unwrap().is_none());
        assert!(parse_line("   \n").unwrap().is_none());
    }
}
