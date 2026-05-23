//! Phase 1 — `screen.*` integration tests against the mock helper.
//!
//! Validates: capability gating, JPEG/PNG output, explicit output_path
//! routing, list_displays response shape.

use std::path::PathBuf;

use corivo_app_lib::services::capture_client::{
    CaptureClient, CaptureError, CaptureScreenOpts, ImageFormat, SpawnOptions,
};
use tempfile::TempDir;

fn mock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_capture_helper_mock"))
}

fn opts() -> SpawnOptions {
    SpawnOptions::new(mock_path(), "0.0.1-test")
}

#[tokio::test]
async fn capture_screen_unsupported_when_capability_off() {
    // No `CORIVO_MOCK_ADVERTISE_SCREEN` → capability stays false.
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    assert!(!client.capabilities().screen_capture);

    let err = client
        .capture_screen(CaptureScreenOpts::jpeg())
        .await
        .err()
        .expect("expected unsupported error");
    match err {
        CaptureError::Unsupported(msg) => assert!(msg.contains("screen.capture")),
        other => panic!("expected Unsupported, got {other:?}"),
    }

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn capture_screen_jpeg_writes_to_explicit_path() {
    let tmp = TempDir::new().expect("tempdir");
    let target = tmp.path().join("frame.jpg");

    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_SCREEN", "1"))
        .await
        .expect("spawn");
    assert!(client.capabilities().screen_capture);

    let shot = client
        .capture_screen(CaptureScreenOpts::jpeg().to_path(&target).with_quality(70))
        .await
        .expect("capture_screen");

    assert_eq!(shot.path, target);
    assert_eq!(shot.width, 16);
    assert_eq!(shot.height, 16);
    assert!(target.exists(), "helper should have written the JPEG");
    assert!(target.metadata().unwrap().len() > 0);

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn capture_screen_png_round_trips() {
    let tmp = TempDir::new().expect("tempdir");
    let target = tmp.path().join("frame.png");

    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_SCREEN", "1"))
        .await
        .expect("spawn");

    let shot = client
        .capture_screen(CaptureScreenOpts::png().to_path(&target))
        .await
        .expect("capture_screen png");

    assert_eq!(shot.path, target);
    assert!(target.exists());
    // First 8 bytes of a PNG file are the PNG signature.
    let bytes = std::fs::read(&target).expect("read png");
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn capture_screen_with_omitted_output_path_uses_temp_dir() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_SCREEN", "1"))
        .await
        .expect("spawn");

    let shot = client
        .capture_screen(CaptureScreenOpts::jpeg())
        .await
        .expect("capture_screen tempdir");

    assert!(shot.path.exists());
    let temp = std::env::temp_dir();
    assert!(
        shot.path.starts_with(&temp),
        "expected temp-dir path, got {}",
        shot.path.display()
    );

    // Cleanup: caller responsibility per protocol contract.
    let _ = std::fs::remove_file(&shot.path);
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn list_displays_returns_two_mock_entries() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_SCREEN", "1"))
        .await
        .expect("spawn");

    let displays = client.list_displays().await.expect("list_displays");
    assert_eq!(displays.len(), 2);
    let main: Vec<_> = displays.iter().filter(|d| d.is_main).collect();
    assert_eq!(main.len(), 1, "exactly one is_main display expected");
    assert_eq!(main[0].id, "mock-display-1");
    assert_eq!(main[0].width, 1920);

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn list_displays_unsupported_without_capability() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .list_displays()
        .await
        .err()
        .expect("expected unsupported");
    match err {
        CaptureError::Unsupported(_) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
    let _ = client.shutdown().await;
}
