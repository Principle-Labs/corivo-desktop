//! Quick Ask hotkey bookkeeping.
//!
//! The actual hotkey is **double-tap ⌥ Option** — implemented in
//! `services/double_tap_hotkey.rs` on top of `NSEvent`'s global
//! `flagsChanged` monitor. We can't use `RegisterEventHotKey` (the API
//! `tauri-plugin-global-shortcut` wraps) because it refuses to bind a
//! bare modifier.
//!
//! Since the binding is fixed in code, this service only carries:
//!
//!  - **what** the binding is (today: a `DoubleTapOption` singleton —
//!    typed so the settings UI doesn't have to hardcode the string), and
//!  - whether `DoubleTapHotkey::install` succeeded at boot.
//!
//! The Settings UI (`capture-privacy-section.tsx`) consults
//! `hotkey_status` to render the binding label and a registration error
//! if the NSEvent monitor failed to install.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyBinding {
    /// Tap ⌥ twice within ~400 ms (see `DOUBLE_TAP_WINDOW` in
    /// `double_tap_hotkey.rs`). Today this is the only supported
    /// binding; the enum exists so the settings UI is forward-compatible
    /// with picking other modifiers later without another schema break.
    DoubleTapOption,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotkeyStatus {
    pub binding: HotkeyBinding,
    /// True once the `NSEvent` global monitor has been installed.
    /// `false` means the boot path called
    /// `addGlobalMonitorForEventsMatchingMask:` and got `nil` back —
    /// usually a permissions / sandboxing problem.
    pub installed: bool,
    /// Set when monitor installation failed; rendered as a red status
    /// card in Settings.
    pub error: Option<String>,
}

impl Default for HotkeyStatus {
    fn default() -> Self {
        Self {
            binding: HotkeyBinding::DoubleTapOption,
            installed: false,
            error: None,
        }
    }
}

#[derive(Default)]
pub struct HotkeyService {
    status: RwLock<HotkeyStatus>,
}

impl HotkeyService {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn current(&self) -> HotkeyStatus {
        self.status.read().await.clone()
    }

    pub async fn record_installed(&self) {
        let mut guard = self.status.write().await;
        guard.installed = true;
        guard.error = None;
    }

    pub async fn record_install_failure(&self, error: impl Into<String>) {
        let mut guard = self.status.write().await;
        guard.installed = false;
        guard.error = Some(error.into());
    }
}

pub type SharedHotkeyService = Arc<HotkeyService>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_service_reports_default_binding_and_not_installed() {
        let svc = HotkeyService::new();
        let status = svc.current().await;
        assert_eq!(status.binding, HotkeyBinding::DoubleTapOption);
        assert!(!status.installed);
        assert!(status.error.is_none());
    }

    #[tokio::test]
    async fn record_installed_clears_previous_error() {
        let svc = HotkeyService::new();
        svc.record_install_failure("addGlobalMonitor returned nil")
            .await;
        assert!(svc.current().await.error.is_some());

        svc.record_installed().await;
        let after = svc.current().await;
        assert!(after.installed);
        assert!(after.error.is_none());
    }

    #[tokio::test]
    async fn record_install_failure_carries_error_message() {
        let svc = HotkeyService::new();
        svc.record_install_failure("nil monitor").await;
        let after = svc.current().await;
        assert!(!after.installed);
        assert_eq!(after.error.as_deref(), Some("nil monitor"));
    }
}
