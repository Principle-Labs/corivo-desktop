//! Phase 5 — `recording.*` typed wrappers around the helper's audio
//! recording. Helper writes segmented multi-track .m4a files directly
//! to `output_dir`; segment-closed events arrive on the shared event
//! bus.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{CaptureClient, DEFAULT_REQUEST_DEADLINE};
use super::error::{CaptureError, Result};
use super::protocol::methods;

#[derive(Debug, Clone)]
pub struct StartRecordingOpts {
    pub session_id: String,
    pub output_dir: PathBuf,
    pub segment_seconds: f64,
    pub capture_system_audio: bool,
    pub capture_microphone: bool,
    pub microphone_device_id: Option<String>,
}

impl StartRecordingOpts {
    pub fn new(session_id: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            output_dir: output_dir.into(),
            segment_seconds: 10.0,
            capture_system_audio: true,
            capture_microphone: true,
            microphone_device_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingHandle {
    pub session_id: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordingStopResult {
    pub session_id: String,
    pub stopped_at: DateTime<Utc>,
    #[serde(default)]
    pub total_segments: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct MicrophoneList {
    devices: Vec<MicrophoneDevice>,
}

impl CaptureClient {
    pub async fn start_recording(&self, opts: StartRecordingOpts) -> Result<RecordingHandle> {
        if !self.capabilities().audio_record {
            return Err(CaptureError::Unsupported(
                "helper does not support recording.start".into(),
            ));
        }
        let mut payload = json!({
            "session_id": opts.session_id,
            "output_dir": opts.output_dir.to_string_lossy(),
            "segment_seconds": opts.segment_seconds,
            "capture_system_audio": opts.capture_system_audio,
            "capture_microphone": opts.capture_microphone,
        });
        if let Some(id) = opts.microphone_device_id {
            payload["microphone_device_id"] = serde_json::Value::String(id);
        }
        // Recording start can take a moment on first call (permission prompt
        // may pop synchronously); allow a longer client-side deadline.
        let deadline = Duration::from_secs(10);
        self.request(methods::RECORDING_START, Some(payload), deadline)
            .await
    }

    pub async fn stop_recording(&self, session_id: &str) -> Result<RecordingStopResult> {
        if !self.capabilities().audio_record {
            return Err(CaptureError::Unsupported(
                "helper does not support recording.stop".into(),
            ));
        }
        let deadline = Duration::from_secs(10);
        self.request(
            methods::RECORDING_STOP,
            Some(json!({ "session_id": session_id })),
            deadline,
        )
        .await
    }

    pub async fn list_microphones(&self) -> Result<Vec<MicrophoneDevice>> {
        if !self.capabilities().audio_record {
            return Err(CaptureError::Unsupported(
                "helper does not support recording.list_microphones".into(),
            ));
        }
        let result: MicrophoneList = self
            .request(
                methods::RECORDING_LIST_MICROPHONES,
                Some(json!({})),
                DEFAULT_REQUEST_DEADLINE,
            )
            .await?;
        Ok(result.devices)
    }
}
