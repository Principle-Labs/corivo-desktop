//! Screen-capture primitives: take a JPEG, query system idle time.
//!
//! **Phase 6 migration**: video frame capture used to go through `xcap`
//! (which wraps `CGWindowListCreateImage` on macOS — deprecated since
//! macOS 14, removed-in-spirit on Tahoe). It now goes through the
//! cross-platform capture helper (see [`crate::services::capture_client`]),
//! which uses ScreenCaptureKit on macOS and WGC on Windows.
//!
//! The function signature is unchanged so callers stay synchronous-feel
//! through [`tokio::task::spawn_blocking`] wrappers, and the existing
//! NSException catch is no longer needed (the helper isolates Objective-C
//! exceptions in its own process).

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::error::{CorivoError, Result};
use crate::services::capture_client::{self, CaptureScreenOpts};

/// JPEG bytes + timestamp + system-idle seconds at the moment of capture.
pub struct ScreenshotCapture {
    pub jpeg: Vec<u8>,
    pub captured_at: DateTime<Utc>,
    pub idle_seconds: Option<u64>,
}

impl ScreenshotCapture {
    /// SHA256 over the JPEG bytes, formatted `sha256:<hex>` for parity with
    /// `content_hash` in the snapshot envelope. Used by the dedup step to
    /// decide "same screen as last frame".
    pub fn screenshot_hash(&self) -> String {
        let digest = Sha256::digest(&self.jpeg);
        format!("sha256:{}", hex::encode(digest))
    }
}

/// Capture the primary monitor as JPEG via the helper sidecar.
///
/// The helper writes the encoded JPEG to a temp file inside its OS temp
/// dir; we read it back and then delete it. For longer-term caching (the
/// live frames pipeline) the consumer copies bytes into the captures store.
pub async fn take_jpeg(jpeg_quality: u8) -> Result<ScreenshotCapture> {
    let captured_at = Utc::now();
    let idle_seconds = current_idle_seconds();

    let client = capture_client::global::get()
        .map_err(|e| CorivoError::Internal(format!("screen_capture: helper unavailable: {e}")))?;

    let opts = CaptureScreenOpts::jpeg().with_quality(jpeg_quality);
    let shot = client
        .capture_screen(opts)
        .await
        .map_err(|e| CorivoError::Internal(format!("screen_capture: helper call failed: {e}")))?;

    let bytes = tokio::fs::read(&shot.path).await.map_err(|e| {
        CorivoError::Internal(format!(
            "screen_capture: read {} from helper: {e}",
            shot.path.display()
        ))
    })?;
    // Best-effort cleanup of the helper's temp file.
    let _ = tokio::fs::remove_file(&shot.path).await;

    Ok(ScreenshotCapture {
        jpeg: bytes,
        captured_at,
        idle_seconds,
    })
}

/// Seconds since last input event (keyboard / mouse). `None` if the system
/// can't tell us (non-macOS, or a CG call that returned NaN).
///
/// This stays in-process even after the screen-capture migration: it's a
/// single CGEventSource API call (zero allocation, zero state) that
/// doesn't justify an IPC round-trip.
#[cfg(target_os = "macos")]
pub fn current_idle_seconds() -> Option<u64> {
    use core_graphics::event_source::CGEventSourceStateID;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            state_id: CGEventSourceStateID,
            event_type: u32,
        ) -> f64;
    }

    // kCGAnyInputEventType — any HID input. The constant isn't generated as
    // a symbol; the documented value is u32::MAX.
    let any_input_event_type = u32::MAX;
    let idle = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            CGEventSourceStateID::HIDSystemState,
            any_input_event_type,
        )
    };
    if idle.is_finite() && idle >= 0.0 {
        Some(idle.floor() as u64)
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
pub fn current_idle_seconds() -> Option<u64> {
    None
}
