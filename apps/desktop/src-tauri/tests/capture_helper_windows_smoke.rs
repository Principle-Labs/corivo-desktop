//! End-to-end smoke for the real Windows C++ capture helper.
//!
//! Counterpart to `capture_helper_macos_smoke.rs`. Same Phase 0 happy-path
//! coverage; the full failure-mode matrix stays on the mock-helper
//! integration suite.
//!
//! Skip semantics: if the helper binary hasn't been built (fresh checkout
//! or non-Windows host), every test in this file emits a one-line skip
//! notice and passes.

#![cfg(target_os = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use corivo_app_lib::services::capture_client::{CaptureClient, HelperHealth, SpawnOptions};

fn helper_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("binaries/corivo-capture-helper-x86_64-pc-windows-msvc.exe")
}

fn require_helper() -> Option<PathBuf> {
    let path = helper_path();
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "skip: windows helper binary not built at {} — run `pnpm --filter @corivo/desktop-helpers build`",
            path.display()
        );
        None
    }
}

fn opts(path: PathBuf) -> SpawnOptions {
    SpawnOptions::new(path, "0.0.1-smoke")
}

#[tokio::test]
async fn handshake_succeeds_and_helper_reports_windows_platform() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");

    let platform = client.platform();
    assert!(
        matches!(
            platform.os,
            corivo_app_lib::services::capture_client::Os::Windows
        ),
        "expected helper to identify as windows, got {:?}",
        platform.os
    );
    assert!(
        !platform.os_version.is_empty(),
        "helper should populate os_version"
    );

    // Phase 0 helper advertises no capabilities — every flag false.
    let caps = client.capabilities();
    assert!(!caps.audio_record);
    assert!(!caps.screen_capture);
    assert!(!caps.ax_query);
    assert!(!caps.ax_events);
    assert!(!caps.foreground_monitor);
    assert!(!caps.ocr_local);

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ping_round_trips_through_real_helper() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    client.ping().await.expect("ping should succeed");
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn heartbeat_marks_helper_alive_within_a_few_seconds() {
    let Some(path) = require_helper() else { return };
    let client =
        CaptureClient::spawn(opts(path).with_env("CORIVO_HELPER_HEARTBEAT_INTERVAL_MS", "200"))
            .await
            .expect("spawn");

    tokio::time::sleep(Duration::from_millis(700)).await;

    assert_eq!(client.health(), HelperHealth::Alive);
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn shutdown_request_completes_cleanly() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    client.shutdown().await.expect("shutdown should succeed");
}

#[tokio::test]
async fn helper_version_is_populated() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    assert_eq!(client.helper_version(), "0.1.0");
    let _ = client.shutdown().await;
}
