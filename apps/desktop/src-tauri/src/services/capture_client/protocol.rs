//! Wire types for the capture-helper v1 protocol.
//!
//! These mirror [`schemas/capture-helper-v1.json`](../../../schemas/capture-helper-v1.json).
//! Keep the two in sync — adding a new method or event means:
//!
//! 1. Add it to the JSON Schema (extending `RequestMethod` / `EventName` enum)
//! 2. Add a constant to the relevant module here (`methods::*` / `events::*`)
//! 3. Define the request / result / event payload as plain serde structs
//!    used by the high-level wrappers in [`super::client`]
//!
//! v1 is **additive only**: existing fields and methods may not change
//! semantics. Breaking changes go through a new protocol number (`v2`).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Discrete protocol number.
///
/// Serialized as the lowercase literal "v1" so the JSON Schema's
/// `RequestMethod` -> `enum: ["v1"]` accepts it as-is. Unknown wire values
/// (e.g. a future helper advertising `v2` to a v1-only client) deserialize
/// to [`ProtocolVersion::Unknown`] so the version-negotiation path can
/// detect "no overlap" cleanly instead of failing the whole handshake at
/// the JSON layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtocolVersion {
    #[serde(rename = "v1")]
    V1,

    /// Forward-compat sentinel — any protocol value not known to this
    /// client. Never serialize this back; it's a deserialize-only state
    /// used during handshake to fall through to the "no shared protocol"
    /// error.
    #[serde(other)]
    Unknown,
}

impl ProtocolVersion {
    /// Versions this client speaks. The handshake picks the first entry
    /// in this list that the helper also lists in its `supported_protocols`.
    /// `Unknown` is intentionally excluded.
    pub const ALL: &'static [ProtocolVersion] = &[ProtocolVersion::V1];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arch {
    #[serde(rename = "arm64")]
    Arm64,
    #[serde(rename = "x86_64")]
    X86_64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: Os,
    pub os_version: String,
    pub arch: Arch,
}

/// Helper-declared feature flags. The Rust client makes feature decisions
/// purely from these — never from `helper_version` / `os_version`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub audio_record: bool,
    #[serde(default)]
    pub audio_per_app: bool,
    #[serde(default)]
    pub screen_capture: bool,
    #[serde(default)]
    pub ax_query: bool,
    #[serde(default)]
    pub ax_events: bool,
    #[serde(default)]
    pub foreground_monitor: bool,
    #[serde(default)]
    pub ocr_local: bool,
    /// Forward-compat: any capability key the client doesn't know lands
    /// here. Surfaced through tracing logs but never gates behavior.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Top-level message envelope. Every NDJSON line is exactly one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello(Hello),
    HelloAck(HelloAck),
    Heartbeat(Heartbeat),
    Log(LogMessage),
    Request(Request),
    Response(Response),
    Event(Event),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub helper_version: String,
    pub supported_protocols: Vec<ProtocolVersion>,
    pub capabilities: Capabilities,
    pub platform: PlatformInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub selected_protocol: ProtocolVersion,
    pub client_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Heartbeat {
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub in_flight_requests: u32,
    #[serde(default)]
    pub recording_sessions: Vec<RecordingSessionStatus>,
    #[serde(default)]
    pub ax_subscriptions: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSessionStatus {
    pub session_id: String,
    pub segments_written: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMessage {
    pub level: LogLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: Uuid,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: Uuid,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    Unsupported,
    PermissionDenied,
    ResourceBusy,
    NotFound,
    Timeout,
    Internal,
    OsError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub name: String,
    pub ts: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// v1 method names. Each new phase appends; existing methods cannot
/// change semantics (only add optional fields).
pub mod methods {
    // Phase 0 — control plane.
    pub const PING: &str = "ping";
    pub const SHUTDOWN: &str = "shutdown";

    // Phase 1 — single-frame screen capture.
    pub const SCREEN_CAPTURE: &str = "screen.capture";
    pub const SCREEN_LIST_DISPLAYS: &str = "screen.list_displays";

    // Phase 2 — synchronous accessibility text query + selection probe.
    pub const AX_QUERY: &str = "ax.query";
    pub const AX_PROBE_SELECTION: &str = "ax.probe_selection";

    // Phase 3 — async subscriptions (events emitted out-of-band).
    pub const AX_SUBSCRIBE: &str = "ax.subscribe";
    pub const AX_UNSUBSCRIBE: &str = "ax.unsubscribe";
    pub const FOREGROUND_SUBSCRIBE: &str = "foreground.subscribe";
    pub const FOREGROUND_UNSUBSCRIBE: &str = "foreground.unsubscribe";
    pub const FOREGROUND_CURRENT: &str = "foreground.current";

    // Phase 4 — local OCR (Vision / Windows.Media.Ocr).
    pub const OCR_RUN: &str = "ocr.run";

    // Phase 5 — meeting audio recording.
    pub const RECORDING_START: &str = "recording.start";
    pub const RECORDING_STOP: &str = "recording.stop";
    pub const RECORDING_LIST_MICROPHONES: &str = "recording.list_microphones";

    // Phase 7 — system permission status (per-bundle on macOS, per-API on Windows).
    pub const PERMISSION_STATUS: &str = "permission.status";
}

/// v1 event names. Each new phase appends.
pub mod event_names {
    pub const FOREGROUND_APP_ACTIVATED: &str = "foreground.app_activated";
    pub const AX_FOCUSED_WINDOW_CHANGED: &str = "ax.focused_window_changed";
    pub const AX_TITLE_CHANGED: &str = "ax.title_changed";
    pub const RECORDING_SEGMENT_CLOSED: &str = "recording.segment_closed";
    pub const RECORDING_ERROR: &str = "recording.error";
}

/// Phase 0 has no domain events. New event names land here in later phases.
pub mod events {
    // (intentionally empty for v1 Phase 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hello_round_trips_through_envelope() {
        let original = Message::Hello(Hello {
            helper_version: "0.1.0".into(),
            supported_protocols: vec![ProtocolVersion::V1],
            capabilities: Capabilities {
                audio_record: true,
                ..Default::default()
            },
            platform: PlatformInfo {
                os: Os::Macos,
                os_version: "14.5.0".into(),
                arch: Arch::Arm64,
            },
        });
        let raw = serde_json::to_value(&original).unwrap();
        assert_eq!(raw["type"], "hello");
        assert_eq!(raw["helper_version"], "0.1.0");
        assert_eq!(raw["supported_protocols"], json!(["v1"]));
        assert_eq!(raw["platform"]["os"], "macos");

        let parsed: Message = serde_json::from_value(raw).unwrap();
        match parsed {
            Message::Hello(h) => assert_eq!(h.helper_version, "0.1.0"),
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    #[test]
    fn response_with_error_serializes_without_result_field() {
        let r = Message::Response(Response {
            id: Uuid::nil(),
            ok: false,
            result: None,
            error: Some(ResponseError {
                code: ErrorCode::Timeout,
                message: "deadline".into(),
                detail: json!(null),
            }),
        });
        let raw = serde_json::to_value(&r).unwrap();
        assert_eq!(raw["type"], "response");
        assert_eq!(raw["ok"], false);
        assert!(
            raw.get("result").is_none(),
            "result should be omitted on error"
        );
        assert_eq!(raw["error"]["code"], "TIMEOUT");
    }

    #[test]
    fn capabilities_preserve_unknown_keys() {
        let raw = json!({
            "audio_record": true,
            "future_flag_we_dont_know_yet": true
        });
        let caps: Capabilities = serde_json::from_value(raw).unwrap();
        assert!(caps.audio_record);
        assert!(caps.extra.contains_key("future_flag_we_dont_know_yet"));
    }

    #[test]
    fn hello_ack_round_trips() {
        let m = Message::HelloAck(HelloAck {
            selected_protocol: ProtocolVersion::V1,
            client_version: "0.1.0".into(),
        });
        let raw = serde_json::to_string(&m).unwrap();
        let parsed: Message = serde_json::from_str(&raw).unwrap();
        assert!(matches!(parsed, Message::HelloAck(_)));
    }

    #[test]
    fn heartbeat_with_defaults_round_trips() {
        let raw = json!({
            "type": "heartbeat",
            "ts": "2026-05-05T12:00:00Z"
        });
        let parsed: Message = serde_json::from_value(raw).unwrap();
        match parsed {
            Message::Heartbeat(h) => {
                assert_eq!(h.in_flight_requests, 0);
                assert!(h.recording_sessions.is_empty());
                assert!(h.ax_subscriptions.is_empty());
            }
            other => panic!("expected Heartbeat, got {other:?}"),
        }
    }
}
