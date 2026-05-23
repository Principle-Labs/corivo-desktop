//! Rust client for the cross-platform capture helper sidecar.
//!
//! See [`apps/desktop/docs/capture-helper-architecture-spec.md`](../../../../docs/capture-helper-architecture-spec.md)
//! for the design rationale and protocol contract. The wire format is
//! NDJSON-over-stdio, defined by
//! [`schemas/capture-helper-v1.json`](../../../schemas/capture-helper-v1.json).
//!
//! Phase 0 (this module) implements the **control plane only**: spawn,
//! handshake, heartbeat, ping, shutdown. Domain methods (`recording.*`,
//! `screen.*`, `ax.*`, `foreground.*`, `ocr.*`) are added in subsequent
//! phases under the same v1 protocol number — additive only.

pub mod ax;
pub mod ax_subscribe;
pub mod client;
pub mod codec;
pub mod error;
pub mod events;
pub mod foreground;
pub mod global;
pub mod health;
pub mod ocr;
pub mod permission;
pub mod process;
pub mod protocol;
pub mod recording;
pub mod router;
pub mod screen;

pub use ax::{AxProbeSelectionResult, AxQueryOpts, AxQueryResult, AxSkipPredicate};
pub use ax_subscribe::AxNotification;
pub use client::{CaptureClient, HelperHealth, HelperSnapshot};
pub use error::{CaptureError, PermissionKind, Result};
pub use events::HelperEvent;
pub use foreground::ForegroundApp;
pub use ocr::{OcrOpts, OcrResult};
pub use permission::{PermissionState, PermissionStatus};
pub use process::SpawnOptions;
pub use protocol::{
    Capabilities, ErrorCode, Event, Hello, HelloAck, LogLevel, LogMessage, Message, Os,
    PlatformInfo, ProtocolVersion, Request, Response, ResponseError,
};
pub use recording::{MicrophoneDevice, RecordingHandle, RecordingStopResult, StartRecordingOpts};
pub use screen::{CaptureScreenOpts, Display, ImageFormat, Screenshot};
