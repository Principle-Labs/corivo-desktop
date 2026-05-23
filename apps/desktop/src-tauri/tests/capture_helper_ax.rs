//! Phase 2 — `ax.*` integration tests against the mock helper.

use std::path::PathBuf;

use corivo_app_lib::services::capture_client::{
    AxQueryOpts, AxSkipPredicate, CaptureClient, CaptureError, SpawnOptions,
};

fn mock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_capture_helper_mock"))
}

fn opts() -> SpawnOptions {
    SpawnOptions::new(mock_path(), "0.0.1-test")
}

#[tokio::test]
async fn ax_query_unsupported_when_capability_off() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    assert!(!client.capabilities().ax_query);

    let err = client
        .ax_query(AxQueryOpts::for_pid(1234))
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(err, CaptureError::Unsupported(_)));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_query_returns_canned_text_for_pid() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_AX", "1"))
        .await
        .expect("spawn");
    assert!(client.capabilities().ax_query);

    let result = client
        .ax_query(AxQueryOpts::for_pid(4321))
        .await
        .expect("ax.query");
    assert!(result.text.contains("[TITLE]"));
    assert!(result.text.contains("4321"));
    assert!(!result.truncated);
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_query_skip_predicate_round_trips_to_helper() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_AX", "1"))
        .await
        .expect("spawn");
    let skip = AxSkipPredicate {
        skip_roles: vec!["AXScrollBar".into()],
        skip_subroles: vec!["AXLandmarkRegion".into()],
        skip_descriptions_substr: vec!["sidebar".into()],
    };
    let _ = client
        .ax_query(AxQueryOpts::for_pid(1).with_skip(skip))
        .await
        .expect("ax.query with skip predicate");
    // Mock doesn't use the predicate, just shouldn't crash on the wire.
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_probe_selection_returns_none_by_default() {
    let client = CaptureClient::spawn(opts().with_env("CORIVO_MOCK_ADVERTISE_AX", "1"))
        .await
        .expect("spawn");

    let sel = client
        .ax_probe_selection(1234)
        .await
        .expect("probe_selection");
    assert!(sel.is_none(), "expected no selection by default");
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_probe_selection_returns_canned_string_when_env_set() {
    let client = CaptureClient::spawn(
        opts()
            .with_env("CORIVO_MOCK_ADVERTISE_AX", "1")
            .with_env("CORIVO_MOCK_AX_SELECTION", "highlighted text"),
    )
    .await
    .expect("spawn");

    let sel = client
        .ax_probe_selection(1234)
        .await
        .expect("probe_selection");
    assert_eq!(sel.as_deref(), Some("highlighted text"));
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn ax_probe_selection_unsupported_when_capability_off() {
    let client = CaptureClient::spawn(opts()).await.expect("spawn");
    let err = client
        .ax_probe_selection(1234)
        .await
        .err()
        .expect("expected unsupported");
    assert!(matches!(err, CaptureError::Unsupported(_)));
    let _ = client.shutdown().await;
}
