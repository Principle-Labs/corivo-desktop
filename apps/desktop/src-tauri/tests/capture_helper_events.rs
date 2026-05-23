//! Phase 3 — `foreground.*` and `ax.subscribe/unsubscribe` integration
//! tests against the mock helper.

use std::path::PathBuf;
use std::time::Duration;

use corivo_app_lib::services::capture_client::{
    AxNotification, CaptureClient, CaptureError, HelperEvent, SpawnOptions,
};
use tokio::time::timeout;

fn mock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_capture_helper_mock"))
}

fn opts() -> SpawnOptions {
    SpawnOptions::new(mock_path(), "0.0.1-test")
}

const EVENT_WAIT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn foreground_subscribe_unsupported_when_capability_off() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .subscribe_foreground()
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(err, CaptureError::Unsupported(_)));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn current_foreground_returns_mock_app() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_FOREGROUND", "1"))
        .await
        .expect("spawn");
    let app = client
        .current_foreground()
        .await
        .expect("current_foreground");
    assert_eq!(app.pid, Some(4321));
    assert_eq!(app.bundle_id.as_deref(), Some("com.mock.app"));
    assert_eq!(app.app_name.as_deref(), Some("Mock App"));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn foreground_subscribe_emits_one_synthetic_event() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_FOREGROUND", "1"))
        .await
        .expect("spawn");
    let mut rx = client.subscribe_events();

    client
        .subscribe_foreground()
        .await
        .expect("subscribe_foreground");

    let evt = timeout(EVENT_WAIT, rx.recv())
        .await
        .expect("event arrived in time")
        .expect("recv");
    match evt {
        HelperEvent::ForegroundAppActivated {
            pid,
            bundle_id,
            app_name,
            ..
        } => {
            assert_eq!(pid, Some(4321));
            assert_eq!(bundle_id.as_deref(), Some("com.mock.app"));
            assert_eq!(app_name.as_deref(), Some("Mock App"));
        }
        other => panic!("expected ForegroundAppActivated, got {other:?}"),
    }

    let _ = client.unsubscribe_foreground().await;
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_subscribe_emits_synthetic_focused_window_event() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_AX_EVENTS", "1"))
        .await
        .expect("spawn");
    let mut rx = client.subscribe_events();

    client
        .subscribe_ax(
            7777,
            &[AxNotification::FocusedWindow, AxNotification::Title],
        )
        .await
        .expect("subscribe_ax");

    let evt = timeout(EVENT_WAIT, rx.recv())
        .await
        .expect("event arrived")
        .expect("recv");
    match evt {
        HelperEvent::AxFocusedWindowChanged {
            pid, window_title, ..
        } => {
            assert_eq!(pid, 7777);
            assert_eq!(window_title.as_deref(), Some("Mock Focused Window"));
        }
        other => panic!("expected AxFocusedWindowChanged, got {other:?}"),
    }

    let _ = client.unsubscribe_ax(7777).await;
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_subscribe_unsupported_when_capability_off() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .subscribe_ax(1234, &[AxNotification::FocusedWindow])
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(err, CaptureError::Unsupported(_)));
    let _ = client.shutdown().await;
}
