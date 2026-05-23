//! Phase 4 (OCR) + Phase 5 (recording) integration tests against the mock
//! helper. The mock fakes both — OCR returns a canned string keyed off the
//! image basename, recording writes a placeholder segment file and emits a
//! single `recording.segment_closed` event.

use std::path::PathBuf;
use std::time::Duration;

use corivo_app_lib::services::capture_client::{
    CaptureClient, HelperEvent, OcrOpts, SpawnOptions, StartRecordingOpts,
};
use tempfile::TempDir;
use tokio::time::timeout;

fn mock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_capture_helper_mock"))
}

fn opts() -> SpawnOptions {
    SpawnOptions::new(mock_path(), "0.0.1-test")
}

const EVENT_WAIT: Duration = Duration::from_secs(3);

// ---------- Phase 4 (OCR) ----------

#[tokio::test]
async fn ocr_run_returns_canned_text_for_basename() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("frame.jpg");
    std::fs::write(&p, b"jpeg-bytes").unwrap();

    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_OCR", "1"))
        .await
        .expect("spawn");
    let result = client
        .run_ocr(OcrOpts::for_path(&p))
        .await
        .expect("ocr.run");
    assert!(result.text.contains("frame.jpg"));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ocr_run_unsupported_when_capability_off() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .run_ocr(OcrOpts::for_path("/tmp/whatever.jpg"))
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(
        err,
        corivo_app_lib::services::capture_client::CaptureError::Unsupported(_)
    ));
    let _ = client.shutdown().await;
}

// ---------- Phase 5 (recording) ----------

#[tokio::test]
async fn recording_start_writes_segment_and_emits_segment_closed_event() {
    let tmp = TempDir::new().unwrap();
    let session_dir = tmp.path().join("session-1");

    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_RECORDING", "1"))
        .await
        .expect("spawn");
    let mut rx = client.subscribe_events();

    let handle = client
        .start_recording(StartRecordingOpts::new("sess-1", &session_dir))
        .await
        .expect("recording.start");
    assert_eq!(handle.session_id, "sess-1");

    // Mock writes the placeholder segment + emits one segment_closed event.
    let evt = timeout(EVENT_WAIT, async {
        loop {
            match rx.recv().await {
                Ok(HelperEvent::RecordingSegmentClosed {
                    session_id,
                    segment_index,
                    path,
                    sys_track_present,
                    mic_track_present,
                    ..
                }) => {
                    return (
                        session_id,
                        segment_index,
                        path,
                        sys_track_present,
                        mic_track_present,
                    );
                }
                Ok(_) => continue,
                Err(_) => panic!("event bus closed"),
            }
        }
    })
    .await
    .expect("segment_closed arrived in time");

    assert_eq!(evt.0, "sess-1");
    assert_eq!(evt.1, 0);
    assert!(PathBuf::from(&evt.2).exists(), "segment file should exist");
    assert!(evt.3, "sys_track_present should default to true");
    assert!(evt.4, "mic_track_present should default to true");

    let stop = client.stop_recording("sess-1").await.expect("stop");
    assert_eq!(stop.session_id, "sess-1");
    assert_eq!(stop.total_segments, 1);

    let _ = client.shutdown().await;
}

#[tokio::test]
async fn list_microphones_returns_two_mock_devices() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_RECORDING", "1"))
        .await
        .expect("spawn");
    let mics = client.list_microphones().await.expect("list_microphones");
    assert_eq!(mics.len(), 2);
    let default_count = mics.iter().filter(|m| m.is_default).count();
    assert_eq!(default_count, 1);
    assert_eq!(mics[0].id, "mock-mic-builtin");
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn recording_unsupported_when_capability_off() {
    let tmp = TempDir::new().unwrap();
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .start_recording(StartRecordingOpts::new("x", tmp.path()))
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(
        err,
        corivo_app_lib::services::capture_client::CaptureError::Unsupported(_)
    ));
    let _ = client.shutdown().await;
}
