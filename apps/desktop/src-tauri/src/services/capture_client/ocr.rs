//! Phase 4 — `ocr.run` typed wrapper. Local OCR via Vision (macOS) /
//! Windows.Media.Ocr (Windows).

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::client::CaptureClient;
use super::error::{CaptureError, Result};
use super::protocol::methods;

/// Default deadline for an OCR pass. macOS Vision on a 5K screenshot
/// runs ~600-1200ms on Apple Silicon; Windows OCR is similar. We set a
/// generous client-side ceiling so we don't time out before the helper.
const DEFAULT_OCR_DEADLINE: Duration = Duration::from_millis(8000);

#[derive(Debug, Clone)]
pub struct OcrOpts {
    pub image_path: std::path::PathBuf,
    /// Languages to try, in priority order. Default: zh-Hans, zh-Hant, en-US
    /// (matches the existing macOS Vision OCR setup).
    pub languages: Vec<String>,
    pub use_language_correction: bool,
}

impl OcrOpts {
    pub fn for_path(path: impl AsRef<Path>) -> Self {
        Self {
            image_path: path.as_ref().to_path_buf(),
            languages: vec!["zh-Hans".into(), "zh-Hant".into(), "en-US".into()],
            use_language_correction: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcrResult {
    pub text: String,
    #[serde(default)]
    pub elapsed_ms: u64,
}

impl CaptureClient {
    pub async fn run_ocr(&self, opts: OcrOpts) -> Result<OcrResult> {
        if !self.capabilities().ocr_local {
            return Err(CaptureError::Unsupported(
                "helper does not support ocr.run".into(),
            ));
        }
        let payload = json!({
            "image_path": opts.image_path.to_string_lossy(),
            "languages": opts.languages,
            "use_language_correction": opts.use_language_correction,
        });
        self.request(methods::OCR_RUN, Some(payload), DEFAULT_OCR_DEADLINE)
            .await
    }
}
