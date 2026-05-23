//! v3 extractor (spec §六): on-device transform from frontmost-window
//! metadata to plaintext, via per-app AX adapters with on-demand OCR
//! fallback.
//!
//! Public surface is intentionally narrow:
//!
//! ```ignore
//! use crate::services::extractor::{extract, ExtractInput};
//!
//! let outcome = extract(&ExtractInput { /* … */ }).await?;
//! ```
//!
//! Strategy choice (AX-first, OCR fallback, skipped) lives entirely
//! inside `dispatcher.rs`. Capture pipeline and snapshot_consumer don't
//! need to know which path ran — only the resulting `ExtractionOutcome`.
//! When OCR fallback fires, `ExtractionOutcome.screenshot` carries the
//! reference to the on-disk image so the caller can put it on the
//! envelope. No image is ever taken on the AX-only path.

pub mod adapters;
pub mod dispatcher;
pub mod error;
pub mod objc_safe;
pub mod output_format;
pub mod selection_probe;

// Both ax_extractor and ocr_extractor are cross-platform: they talk to
// the capture-helper sidecar via the capture_client singleton. The
// helper itself supplies the platform-specific implementations
// (ScreenCaptureKit + Vision on macOS; WGC + Windows.Media.Ocr on
// Windows). If the helper hasn't spawned yet (degraded boot), both
// modules return a helper-unavailable error which the dispatcher
// translates into a `Skipped` outcome.
pub mod ax_extractor;
pub mod ocr_extractor;

pub use adapters::{AdapterContext, AdapterOutcome, AdapterRegistry, FocusAdapter};
pub use dispatcher::{extract, ExtractInput, ExtractionOutcome, OcrResources};
pub use error::{ExtractorError, Result};
