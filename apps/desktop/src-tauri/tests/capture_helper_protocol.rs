//! Integration tests for the capture-helper v1 protocol.
//!
//! These exercise the full Rust client stack — process spawn, NDJSON
//! framing, hello handshake, in-flight RPC routing, health monitor — by
//! talking to the `capture_helper_mock` binary at `src/bin/`. The mock
//! deliberately implements only the control plane (hello / heartbeat /
//! ping / shutdown) so this suite catches transport / protocol regressions
//! independent of any platform-native capture code.
//!
//! The mock supports test hooks via env vars (see its module docs). Each
//! test case sets the hooks it needs through `SpawnOptions::with_env`.

use std::path::PathBuf;
use std::time::Duration;

use corivo_app_lib::services::capture_client::{
    CaptureClient, CaptureError, HelperHealth, SpawnOptions,
};

/// Cargo wires `CARGO_BIN_EXE_<bin-name>` for every `src/bin/*.rs` so
/// integration tests can locate the just-built binary without hardcoded
/// paths.
fn mock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_capture_helper_mock"))
}

fn opts() -> SpawnOptions {
    SpawnOptions::new(mock_path(), "0.0.1-test")
}

#[tokio::test]
async fn handshake_succeeds_and_capabilities_default_to_false() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let caps = client.capabilities();
    assert!(!caps.audio_record);
    assert!(!caps.screen_capture);
    assert!(!caps.ax_query);
    assert!(!caps.foreground_monitor);
    assert!(!caps.ocr_local);
    assert_eq!(client.helper_version(), env!("CARGO_PKG_VERSION"));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ping_succeeds_through_round_trip() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    client.ping().await.expect("ping should succeed");
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ping_propagates_helper_internal_error() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_REJECT_PING", "1"))
        .await
        .expect("spawn");
    match client.ping().await {
        Err(CaptureError::HelperError { code, message, .. }) => {
            assert_eq!(
                code,
                corivo_app_lib::services::capture_client::ErrorCode::Internal
            );
            assert!(
                message.contains("rejected"),
                "expected the helper's message to mention rejection, got: {message}"
            );
        }
        other => panic!("expected HelperError, got {other:?}"),
    }
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ping_times_out_when_helper_delays_beyond_deadline() {
    // Ping's per-call deadline is 5s; a 7s mock delay overruns it.
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_DELAY_PING_MS", "7000"))
        .await
        .expect("spawn");

    let start = std::time::Instant::now();
    let result = client.ping().await;
    let elapsed = start.elapsed();

    match result {
        Err(CaptureError::Timeout(d)) => {
            assert_eq!(d, Duration::from_secs(5));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
    // Should have returned at the deadline, not waited for the helper.
    assert!(
        elapsed < Duration::from_secs(6),
        "deadline should have fired before mock's 7s reply; elapsed={elapsed:?}"
    );
    // After timeout the router slot should be free again.
    drop(client);
}

#[tokio::test]
async fn shutdown_completes_cleanly() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn helper_crash_during_request_yields_helper_crashed_error() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_EXIT_ON_PING", "1"))
        .await
        .expect("spawn");
    match client.ping().await {
        Err(CaptureError::HelperCrashed) => {}
        other => panic!("expected HelperCrashed, got {other:?}"),
    }
    // Subsequent operations should also fail; helper is gone.
    match client.ping().await {
        Err(CaptureError::HelperUnavailable) | Err(CaptureError::HelperCrashed) => {}
        other => panic!("expected HelperUnavailable/Crashed on second call, got {other:?}"),
    }
}

#[tokio::test]
async fn handshake_times_out_when_helper_skips_hello() {
    let err = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_NO_HELLO", "1"))
        .await
        .err()
        .expect("expected handshake to fail");
    match err {
        CaptureError::Timeout(d) => {
            assert_eq!(d, Duration::from_secs(5));
        }
        other => panic!("expected handshake Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn handshake_rejects_bad_first_message() {
    let err = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_BAD_FIRST_MSG", "1"))
        .await
        .err()
        .expect("expected handshake to fail");
    match err {
        CaptureError::Protocol(msg) => {
            assert!(
                msg.to_lowercase().contains("hello"),
                "protocol error should mention hello expectation, got: {msg}"
            );
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn handshake_rejects_when_no_protocol_overlap() {
    let err = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_NO_PROTOCOL_OVERLAP", "1"))
        .await
        .err()
        .expect("expected handshake to fail");
    match err {
        CaptureError::Protocol(msg) => {
            assert!(
                msg.to_lowercase().contains("protocol"),
                "expected protocol-mismatch error, got: {msg}"
            );
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn health_reports_alive_after_first_heartbeat() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_FAST_HEARTBEAT", "1"))
        .await
        .expect("spawn");

    // Right after spawn, no heartbeat received yet.
    assert!(matches!(
        client.health(),
        HelperHealth::Pending | HelperHealth::Alive
    ));

    // Mock's fast cadence is 50ms; wait a few cycles.
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(client.health(), HelperHealth::Alive);
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn helper_version_is_populated_from_hello() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let v = client.helper_version();
    assert!(
        !v.is_empty(),
        "helper version should be populated from hello"
    );
    // Mock helper reports its own crate version (== this crate's version).
    assert_eq!(v, env!("CARGO_PKG_VERSION"));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn protocol_version_is_v1() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    assert_eq!(
        client.protocol_version(),
        corivo_app_lib::services::capture_client::ProtocolVersion::V1
    );
    let _ = client.shutdown().await;
}
