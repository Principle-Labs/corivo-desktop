//! Phase 1 — `screen.*` typed wrappers around the helper's screen-capture
//! methods. Bound to the `screen_capture` capability.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::client::{CaptureClient, DEFAULT_REQUEST_DEADLINE};
use super::error::{CaptureError, Result};
use super::protocol::methods;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Default)]
pub struct CaptureScreenOpts {
    /// Display id from `list_displays()`. `None` → primary display.
    pub display_id: Option<String>,
    /// If supplied, helper writes the encoded image directly here.
    /// If `None`, helper picks a temp path inside its OS temp dir;
    /// caller is responsible for cleaning that file up.
    pub output_path: Option<PathBuf>,
    pub format: ImageFormat,
    /// JPEG quality 0-100; ignored for PNG.
    pub quality: Option<u8>,
}

impl CaptureScreenOpts {
    pub fn jpeg() -> Self {
        Self {
            format: ImageFormat::Jpeg,
            quality: Some(80),
            ..Self::default()
        }
    }

    pub fn png() -> Self {
        Self {
            format: ImageFormat::Png,
            ..Self::default()
        }
    }

    pub fn to_path(mut self, path: impl AsRef<Path>) -> Self {
        self.output_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn for_display(mut self, display_id: impl Into<String>) -> Self {
        self.display_id = Some(display_id.into());
        self
    }

    pub fn with_quality(mut self, q: u8) -> Self {
        self.quality = Some(q);
        self
    }
}

impl Default for ImageFormat {
    fn default() -> Self {
        ImageFormat::Jpeg
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Screenshot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub captured_at: DateTime<Utc>,
    #[serde(default)]
    pub display_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_main: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DisplayListResult {
    displays: Vec<Display>,
}

impl CaptureClient {
    /// Capture a single frame from a display. Requires the helper to
    /// advertise `screen_capture: true` (Phase 0 / mock helper does not).
    pub async fn capture_screen(&self, opts: CaptureScreenOpts) -> Result<Screenshot> {
        if !self.capabilities().screen_capture {
            return Err(CaptureError::Unsupported(
                "helper does not support screen.capture".into(),
            ));
        }
        let mut payload = serde_json::json!({
            "format": match opts.format {
                ImageFormat::Jpeg => "jpeg",
                ImageFormat::Png => "png",
            },
        });
        if let Some(id) = opts.display_id {
            payload["display_id"] = serde_json::Value::String(id);
        }
        if let Some(path) = opts.output_path {
            payload["output_path"] = serde_json::Value::String(path.to_string_lossy().into_owned());
        }
        if let Some(q) = opts.quality {
            payload["quality"] = serde_json::Value::Number(q.into());
        }
        self.request(
            methods::SCREEN_CAPTURE,
            Some(payload),
            DEFAULT_REQUEST_DEADLINE,
        )
        .await
    }

    /// List displays the helper sees.
    pub async fn list_displays(&self) -> Result<Vec<Display>> {
        if !self.capabilities().screen_capture {
            return Err(CaptureError::Unsupported(
                "helper does not support screen.list_displays".into(),
            ));
        }
        let result: DisplayListResult = self
            .request(
                methods::SCREEN_LIST_DISPLAYS,
                Some(serde_json::json!({})),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(result.displays)
    }
}
