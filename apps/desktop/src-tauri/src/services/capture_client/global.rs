//! Process-global handle to the capture helper client.
//!
//! Phase 6 wiring: at boot, [`crate::lib::run`] spawns the helper sidecar
//! and (on success) installs the client here. Existing modules
//! (`screen_capture`, `ax_extractor`, `selection_probe`, `ocr_extractor`)
//! reach in via [`get`] / [`try_get`] instead of plumbing the client
//! through every constructor — that keeps the migration blast radius
//! contained while still enforcing the helper boundary.
//!
//! Tests that exercise the migrated modules without a real helper should
//! call [`install_for_test`] / [`reset_for_test`] explicitly.

use std::sync::{Arc, OnceLock, RwLock};

use super::client::CaptureClient;
use super::error::{CaptureError, Result};

static GLOBAL: OnceLock<RwLock<Option<Arc<CaptureClient>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<CaptureClient>>> {
    GLOBAL.get_or_init(|| RwLock::new(None))
}

/// Install the helper client at boot. Idempotent — a re-install (e.g.
/// after a forced restart) replaces the previous handle.
pub fn install(client: Arc<CaptureClient>) {
    *slot()
        .write()
        .expect("capture_client::global RwLock poisoned") = Some(client);
}

/// Take the helper offline (drops the client). Used during graceful
/// shutdown so subsequent calls to [`get`] surface a clean
/// `HelperUnavailable` instead of a dangling client.
pub fn uninstall() {
    *slot()
        .write()
        .expect("capture_client::global RwLock poisoned") = None;
}

/// Returns `None` if no helper has been installed (degraded boot, or
/// host-less unit tests).
pub fn try_get() -> Option<Arc<CaptureClient>> {
    slot()
        .read()
        .expect("capture_client::global RwLock poisoned")
        .clone()
}

/// Like [`try_get`] but converts the absence into [`CaptureError::HelperUnavailable`].
/// Most callers want this — they can't proceed without a client and the
/// error type already maps cleanly to the user-visible "capture pipeline
/// degraded" state.
pub fn get() -> Result<Arc<CaptureClient>> {
    try_get().ok_or(CaptureError::HelperUnavailable)
}

/// Test hook: installs an arbitrary client. Public under `test-support`
/// (which `default-features = ["test-support"]` keeps on for cargo test).
#[cfg(any(test, feature = "test-support"))]
pub fn install_for_test(client: Arc<CaptureClient>) {
    install(client)
}

/// Test hook: clears the global so the next `get()` errors with
/// `HelperUnavailable`.
#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_test() {
    uninstall()
}
