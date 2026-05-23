//! Phase 7 — `permission.status` typed wrapper. Used by the settings
//! UI to render "Accessibility / Microphone / Screen Recording: granted
//! / denied / undetermined" without round-tripping platform-specific
//! permission APIs to the Rust side.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::client::CaptureClient;
use super::error::Result;
use super::protocol::methods;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionState {
    Granted,
    Denied,
    Undetermined,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionStatus {
    #[serde(default = "PermissionState::granted_default")]
    pub microphone: PermissionState,
    #[serde(default = "PermissionState::granted_default")]
    pub screen_recording: PermissionState,
    #[serde(default = "PermissionState::granted_default")]
    pub accessibility: PermissionState,
}

impl PermissionState {
    fn granted_default() -> PermissionState {
        // For platforms where a permission isn't a meaningful concept
        // (UIA on Windows, etc.) we treat absence as "granted" — the
        // caller's UI shows nothing instead of a misleading prompt.
        PermissionState::Granted
    }

    pub fn is_granted(&self) -> bool {
        matches!(self, PermissionState::Granted)
    }
}

impl CaptureClient {
    /// Snapshot the platform-level permissions the helper bundle owns.
    /// Cheap (no UI prompt, no dialog) — safe to call from a settings
    /// page render path.
    pub async fn permission_status(&self) -> Result<PermissionStatus> {
        self.request(
            methods::PERMISSION_STATUS,
            Some(json!({})),
            Duration::from_secs(2),
        )
        .await
    }
}
