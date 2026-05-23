//! Error surface for the v3 extractor (spec §六).
//!
//! Independent of `observation_ingest::ObservationIngestError` because the v3
//! extractor is not part of the LLM-vision ingest pipeline — it's a pure
//! on-device transform from screenshot path to text. Failures here are I/O,
//! Vision-framework errors, or AX permission/timeout signals.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractorError {
    /// Screenshot file does not exist on disk.
    #[error("screenshot file missing: {0}")]
    MissingScreenshot(PathBuf),

    /// I/O error reading the screenshot file.
    #[error("io reading screenshot: {0}")]
    Io(#[source] std::io::Error),

    /// On-device extraction (AX or Vision OCR) returned an error. Free-form
    /// string because the underlying APIs surface heterogeneous shapes
    /// (NSError, kAXError codes, timeouts) that don't fit a closed enum.
    #[error("on-device extraction failed: {0}")]
    Failed(String),

    /// Internal plumbing error (spawn_blocking panic, etc.).
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, ExtractorError>;
