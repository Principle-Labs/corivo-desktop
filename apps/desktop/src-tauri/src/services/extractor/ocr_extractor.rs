//! OCR text extraction.
//!
//! **Phase 6 migration**: this module used to call into Apple Vision via
//! `objc2-vision` directly. Vision now lives in the `corivo-capture-helper`
//! sidecar (see [`crate::services::capture_client::ocr`] +
//! `packages/desktop-helpers/macos/Sources/CorivoCaptureHelper/OCR/`). The
//! function signature stays the same so the dispatcher doesn't need to
//! change; the implementation is now an IPC round-trip.
//!
//! Languages and `usesLanguageCorrection` are pinned in the helper at
//! `["zh-Hans", "zh-Hant", "en-US"]` / `false`, matching the behaviour the
//! Rust impl had before this migration.

use std::path::Path;

use crate::services::capture_client::{self, OcrOpts};

use super::error::{ExtractorError, Result};

/// Run local OCR against the screenshot at `path`. Returns the recognized
/// text (reading-order stitched).
///
/// Errors:
/// - `ExtractorError::MissingScreenshot` — file not found
/// - `ExtractorError::Internal(...)` — helper unavailable / IPC error /
///   helper-side OS error
pub async fn extract(path: &Path) -> Result<String> {
    if !path.exists() {
        return Err(ExtractorError::MissingScreenshot(path.to_path_buf()));
    }

    let bytes_in = tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let started = std::time::Instant::now();

    let client = capture_client::global::get()
        .map_err(|e| ExtractorError::Internal(format!("ocr: capture helper unavailable: {e}")))?;

    let result = client
        .run_ocr(OcrOpts::for_path(path))
        .await
        .map_err(|e| ExtractorError::Internal(format!("ocr: helper call failed: {e}")))?;

    tracing::info!(
        bytes_in,
        text_len = result.text.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        helper_elapsed_ms = result.elapsed_ms,
        strategy = "vision_ocr_via_helper",
        "extractor.ocr.done"
    );

    Ok(result.text)
}
