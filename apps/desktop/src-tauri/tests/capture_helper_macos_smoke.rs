//! End-to-end smoke for the real macOS Swift capture helper.
//!
//! These tests exercise the same protocol surface as
//! `capture_helper_protocol.rs` but against the **actual** Swift binary
//! built by `packages/desktop-helpers/macos/build.sh`. Their job is to catch
//! Swift-side regressions that the mock-helper suite can't see (different
//! JSONEncoder casing, missing field, async ordering bug, etc.).
//!
//! The full failure-mode matrix stays on the mock helper (which we can
//! drive into arbitrary states via env hooks). Here we only assert the
//! happy paths of the Phase 0 control plane.
//!
//! ## Skip behavior
//!
//! If the helper binary hasn't been built yet (fresh checkout, CI without
//! the Swift step), every test in this file emits a one-line skip notice
//! and passes. This lets `cargo test` succeed end-to-end before Slice C
//! adds the Swift build to the workspace's pre-test pipeline.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use corivo_app_lib::services::capture_client::{CaptureClient, HelperHealth, SpawnOptions};

/// Resolves to `apps/desktop/src-tauri/binaries/corivo-capture-helper-universal-apple-darwin`.
fn helper_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("binaries/corivo-capture-helper-universal-apple-darwin")
}

/// Per-test gate: if the helper hasn't been built we skip rather than
/// fail, so the wider `cargo test` run stays green on a fresh checkout.
fn require_helper() -> Option<PathBuf> {
    let path = helper_path();
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "skip: macos helper binary not built at {} — run `pnpm --filter @corivo/desktop-helpers build`",
            path.display()
        );
        None
    }
}

fn opts(path: PathBuf) -> SpawnOptions {
    SpawnOptions::new(path, "0.0.1-smoke")
}

#[tokio::test]
async fn handshake_succeeds_and_helper_reports_macos_platform() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");

    let platform = client.platform();
    assert!(
        matches!(
            platform.os,
            corivo_app_lib::services::capture_client::Os::Macos
        ),
        "expected helper to identify as macos, got {:?}",
        platform.os
    );
    assert!(
        !platform.os_version.is_empty(),
        "helper should populate os_version from ProcessInfo"
    );

    // Capabilities reflect what each phase has shipped. By Phase 7 the
    // macOS helper advertises everything: audio recording, screen
    // capture (14.0+), AX text + events, foreground monitor, OCR.
    let caps = client.capabilities();
    // Phase 5 — meeting audio recording.
    assert!(caps.audio_record);
    // Phase 1 — screen capture (14.0+ for SCScreenshotManager).
    let major = host_major_version();
    if major >= 14 {
        assert!(
            caps.screen_capture,
            "macOS {major}+ helper should advertise screen_capture"
        );
    } else {
        assert!(
            !caps.screen_capture,
            "macOS {major} (< 14) helper should not advertise screen_capture"
        );
    }
    // Phase 2 — synchronous AX text walk + selection probe.
    assert!(caps.ax_query);
    // Phase 3 + 7 — foreground monitor + AX events (AXObserver on
    // dedicated CFRunLoop thread).
    assert!(caps.ax_events);
    assert!(caps.foreground_monitor);
    // Phase 4 — Vision OCR.
    assert!(caps.ocr_local);

    let _ = client.shutdown().await;
}

/// Major macOS version of the host running the test, parsed from
/// `sw_vers -productVersion`. Falls back to 14 if the parse fails so the
/// test stays meaningful in unusual environments.
fn host_major_version() -> u32 {
    use std::process::Command;
    let output = Command::new("sw_vers").arg("-productVersion").output().ok();
    let Some(out) = output else { return 14 };
    let s = String::from_utf8_lossy(&out.stdout);
    s.split('.')
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14)
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

    // Tighten the helper's heartbeat cadence so the test doesn't have to
    // wait the spec default (5s). 200ms is plenty to observe the
    // transition without slowing CI noticeably.
    let client =
        CaptureClient::spawn(opts(path).with_env("CORIVO_HELPER_HEARTBEAT_INTERVAL_MS", "200"))
            .await
            .expect("spawn");

    // Allow a couple of cycles of the tightened cadence.
    tokio::time::sleep(Duration::from_millis(700)).await;

    assert_eq!(
        client.health(),
        HelperHealth::Alive,
        "helper should be Alive after heartbeats arrive"
    );

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn shutdown_request_completes_cleanly() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    // shutdown() consumes self; takes whatever cleanup grace period the
    // client's spec mandates.
    client.shutdown().await.expect("shutdown should succeed");
}

#[tokio::test]
async fn helper_version_is_populated() {
    let Some(path) = require_helper() else { return };
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    let v = client.helper_version();
    assert!(
        !v.is_empty(),
        "helper version string should be populated from hello"
    );
    // Helper currently declares "0.1.0" — keep this as a literal so a
    // bump on the helper side is a deliberate test update.
    assert_eq!(v, "0.1.0");
    let _ = client.shutdown().await;
}

/// Phase 4 regression — verifies the UInt64 encoding path through
/// AnyCodable. Vision's OCR response carries `elapsed_ms: UInt64`; an
/// earlier helper build threw `EncodingError.invalidValue: unsupported
/// value of type UInt64` because the AnyCodable encoder only listed
/// `Int` / `Int64` / `Double` branches. We feed a tiny PNG (no text)
/// into ocr.run and assert the typed response decodes — empty `text`
/// is fine, what we're checking is that the wire round-trip succeeded.
#[tokio::test]
async fn ocr_run_encoding_handles_uint64_elapsed_ms() {
    use corivo_app_lib::services::capture_client::OcrOpts;
    use std::io::Write;

    let Some(path) = require_helper() else { return };
    let tmp = std::env::temp_dir().join(format!(
        "corivo-ocr-uint64-smoke-{}.png",
        uuid::Uuid::new_v4()
    ));
    // 16x16 single-pixel PNG: encode via the `image` crate so we don't
    // need to hand-roll a PNG header.
    {
        use image::{ImageBuffer, Rgba};
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 16, Rgba([255, 255, 255, 255]));
        img.save(&tmp).expect("png write");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&tmp)
            .expect("open png for trailing flush");
        f.flush().ok();
    }

    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");
    let result = client
        .run_ocr(OcrOpts::for_path(&tmp))
        .await
        .expect("run_ocr should round-trip without UInt64 encoding errors");
    // `text` may legitimately be empty for a blank PNG; what we care
    // about is that the response decoded — i.e. `elapsed_ms` was a
    // valid JSON number on the wire.
    assert!(result.text.is_empty() || result.text.len() < 256);

    let _ = std::fs::remove_file(&tmp);
    let _ = client.shutdown().await;
}

/// Phase 1 — best-effort smoke that the real Swift helper can enumerate
/// displays via SCShareableContent. This requires Screen Recording
/// permission for the helper binary; on a machine that hasn't granted it,
/// SCShareableContent.current returns an OS error which we report as a
/// skip rather than a failure (the rest of the protocol is still healthy).
#[tokio::test]
async fn list_displays_returns_at_least_one_display() {
    use corivo_app_lib::services::capture_client::CaptureError;

    let Some(path) = require_helper() else { return };
    if host_major_version() < 14 {
        eprintln!(
            "skip: macOS {} predates SCScreenshotManager (needs 14+)",
            host_major_version()
        );
        return;
    }
    let client = CaptureClient::spawn(opts(path)).await.expect("spawn");

    match client.list_displays().await {
        Ok(displays) => {
            assert!(
                !displays.is_empty(),
                "host has at least one display (the one running the test)"
            );
            assert_eq!(
                displays.iter().filter(|d| d.is_main).count(),
                1,
                "exactly one display should be flagged is_main"
            );
        }
        Err(CaptureError::HelperError { code, message, .. }) => {
            eprintln!(
                "skip: list_displays failed (likely missing Screen Recording permission): {code:?} {message}"
            );
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }

    let _ = client.shutdown().await;
}
