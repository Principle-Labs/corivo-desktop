//! Public error type for the capture client.
//!
//! Distinct from [`crate::error::CorivoError`] so callers can pattern-match
//! on capture-specific failure modes (permission denial, capability missing,
//! helper crash) without fighting the catch-all `Internal` bucket.

use std::time::Duration;

use serde_json::Value;

use super::protocol::ErrorCode;

#[derive(thiserror::Error, Debug)]
pub enum CaptureError {
    /// Helper process never started, or has crashed and not yet been restarted.
    #[error("helper not started or has crashed")]
    HelperUnavailable,

    /// Helper crashed while a request was in flight. The request will be
    /// resubmitted by the caller if appropriate; the client itself does not
    /// auto-retry to keep semantics observable.
    #[error("helper crashed during request")]
    HelperCrashed,

    /// stdin/stdout transport failure (read/write error, framing violation).
    #[error("transport: {0}")]
    Transport(String),

    /// Helper sent a structured error response. `code` is one of the v1
    /// `ErrorCode` enum values; `message` is human-readable; `detail` is
    /// method-specific extra context (e.g. OS error code for `OS_ERROR`).
    #[error("helper error: {code:?} - {message}")]
    HelperError {
        code: ErrorCode,
        message: String,
        detail: Value,
    },

    /// A system permission (microphone / screen recording / accessibility) is
    /// required but not granted. UI should drive the onboarding flow when this
    /// fires.
    #[error("permission denied: {kind:?}")]
    PermissionDenied { kind: PermissionKind },

    /// The helper does not advertise the capability needed to fulfill this
    /// call. Callers should check [`super::client::CaptureClient::capabilities`]
    /// before requesting capability-gated functionality.
    #[error("capability unsupported: {0}")]
    Unsupported(String),

    /// Per-call deadline elapsed without a response.
    #[error("timeout after {0:?}")]
    Timeout(Duration),

    /// Protocol violation (helper sent unexpected message type during
    /// handshake, response with no matching request, etc.).
    #[error("protocol: {0}")]
    Protocol(String),

    /// Catch-all for client-side issues that don't map elsewhere.
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    Microphone,
    ScreenRecording,
    Accessibility,
}

pub type Result<T> = std::result::Result<T, CaptureError>;

impl CaptureError {
    /// Translate a v1 `ResponseError` into the strongly-typed client error,
    /// promoting `PERMISSION_DENIED` to its dedicated variant when the detail
    /// shape lets us identify the kind.
    pub(super) fn from_response_error(error: super::protocol::ResponseError) -> Self {
        if error.code == ErrorCode::PermissionDenied {
            if let Some(kind) = parse_permission_kind(&error.detail) {
                return CaptureError::PermissionDenied { kind };
            }
        }
        CaptureError::HelperError {
            code: error.code,
            message: error.message,
            detail: error.detail,
        }
    }
}

fn parse_permission_kind(detail: &Value) -> Option<PermissionKind> {
    let kind_str = detail.get("kind")?.as_str()?;
    match kind_str {
        "microphone" => Some(PermissionKind::Microphone),
        "screen_recording" => Some(PermissionKind::ScreenRecording),
        "accessibility" => Some(PermissionKind::Accessibility),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permission_denied_with_known_kind_promotes_to_typed_variant() {
        let raw = super::super::protocol::ResponseError {
            code: ErrorCode::PermissionDenied,
            message: "no mic".into(),
            detail: json!({ "kind": "microphone" }),
        };
        match CaptureError::from_response_error(raw) {
            CaptureError::PermissionDenied {
                kind: PermissionKind::Microphone,
            } => {}
            other => panic!("expected PermissionDenied(Microphone), got {other:?}"),
        }
    }

    #[test]
    fn permission_denied_without_kind_falls_through_to_helper_error() {
        let raw = super::super::protocol::ResponseError {
            code: ErrorCode::PermissionDenied,
            message: "no mic".into(),
            detail: json!({}),
        };
        match CaptureError::from_response_error(raw) {
            CaptureError::HelperError { code, .. } => assert_eq!(code, ErrorCode::PermissionDenied),
            other => panic!("expected HelperError, got {other:?}"),
        }
    }

    #[test]
    fn other_codes_map_to_helper_error() {
        let raw = super::super::protocol::ResponseError {
            code: ErrorCode::Timeout,
            message: "deadline".into(),
            detail: json!(null),
        };
        match CaptureError::from_response_error(raw) {
            CaptureError::HelperError { code, .. } => assert_eq!(code, ErrorCode::Timeout),
            other => panic!("expected HelperError, got {other:?}"),
        }
    }
}
