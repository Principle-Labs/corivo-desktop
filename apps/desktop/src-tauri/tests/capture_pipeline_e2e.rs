//! End-to-end exercise for the snapshot ingest path.
//!
//! Build a SnapshotEnvelope shaped exactly like what
//! `capture_pipeline::tick` would emit (adapter side-channel + Trigger
//! variants both populated), hand it to LocalConsumer, then read back
//! through the repo and assert every v500 column landed.
//!
//! This is the integration counterpart to the per-module unit tests in
//! `services::snapshot_consumer::local::tests` — those cover specific
//! mappings; this one catches regressions where a column moves between
//! repo / consumer / envelope and only the unit tests on the moved
//! side notice.
//!
//! NOTE: the `capture_pipeline::tick` itself can't run in a host-less
//! integration test (it screen-captures via xcap and probes
//! NSWorkspace). Hence the manual envelope construction here.

use std::sync::Arc;

use chrono::Utc;
use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::create_pool,
        repos::frames::{FrameRepo, SqliteFrameRepo},
    },
    domain::snapshot_envelope::{
        AppInfo, ExtractionResult, ExtractionStrategy, FrameSnapshot, RawDebug, ScreenshotRef,
        SnapshotEnvelope, Trigger, WindowInfo, ENVELOPE_VERSION,
    },
    services::snapshot_consumer::{LocalConsumer, SnapshotConsumer},
};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn snapshot_envelope_round_trips_with_adapter_and_trigger_columns() {
    let dir = TempDir::new().expect("tempdir");
    let pool = create_pool(&dir.path().join("frames.sqlite")).expect("pool");
    let conn = pool.get().expect("conn");
    apply_migrations(&conn).expect("schema");
    drop(conn);

    let frames: Arc<dyn FrameRepo> = Arc::new(SqliteFrameRepo::new(pool));
    let consumer = LocalConsumer::new(frames.clone());

    let captured_at = Utc::now();
    let env = SnapshotEnvelope {
        v: ENVELOPE_VERSION,
        captured_at,
        device_id: "dev-test".into(),
        session_id: "sess-test".into(),
        frame: FrameSnapshot {
            app: AppInfo {
                bundle_id: Some("com.google.Chrome".into()),
                name: Some("Google Chrome".into()),
                pid: Some(123),
            },
            window: WindowInfo {
                title: Some("rust-lang.org".into()),
                is_focused: true,
            },
            url: None,
            screenshot: Some(ScreenshotRef {
                path: "sess-test/0001.jpg".into(),
                hash: "0".repeat(64),
                size_bytes: Some(2048),
            }),
        },
        extraction: ExtractionResult {
            strategy: ExtractionStrategy::Ax,
            ax_text: Some("Welcome to rust-lang.org".into()),
            ocr_text: None,
            adapter_name: Some("chrome".into()),
            adapter_payload: Some(json!({
                "page_title": "rust-lang.org",
                "raw_window_title": "rust-lang.org - Google Chrome",
            })),
            selection: None,
            duration_ms: 42,
            fallback_reason: None,
        },
        trigger: Trigger::FocusChange,
        raw: RawDebug::default(),
    };

    consumer.ingest(env).await.expect("ingest envelope");

    // Read back and assert each v500 field round-tripped.
    let stored = frames
        .last_in_session("sess-test")
        .await
        .expect("repo read")
        .expect("frame inserted");

    assert_eq!(stored.app_bundle_id.as_deref(), Some("com.google.Chrome"));
    assert_eq!(stored.app_name.as_deref(), Some("Google Chrome"));
    assert_eq!(stored.window_title.as_deref(), Some("rust-lang.org"));
    assert_eq!(stored.extraction_strategy, "ax");
    assert_eq!(stored.ax_text.as_deref(), Some("Welcome to rust-lang.org"));
    assert!(stored.ocr_text.is_none());
    assert_eq!(stored.adapter_name.as_deref(), Some("chrome"));
    let payload_json = stored
        .adapter_payload
        .as_deref()
        .expect("payload roundtrip");
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).expect("payload is valid json");
    assert_eq!(payload["page_title"], "rust-lang.org");
    assert_eq!(stored.trigger, "focus_change");
    assert_eq!(stored.extraction_duration_ms, Some(42));
    assert_eq!(stored.screenshot_size_bytes, Some(2048));
}

#[tokio::test]
async fn quick_ask_trigger_is_persisted_distinctly_from_others() {
    let dir = TempDir::new().expect("tempdir");
    let pool = create_pool(&dir.path().join("frames.sqlite")).expect("pool");
    let conn = pool.get().expect("conn");
    apply_migrations(&conn).expect("schema");
    drop(conn);

    let frames: Arc<dyn FrameRepo> = Arc::new(SqliteFrameRepo::new(pool));
    let consumer = LocalConsumer::new(frames.clone());

    for (i, trigger) in [Trigger::Manual, Trigger::QuickAsk, Trigger::FocusChange]
        .iter()
        .enumerate()
    {
        let env = sample_envelope(format!("sess-{i}"), *trigger);
        consumer.ingest(env).await.expect("ingest");
    }

    // Distinct sessions → distinct rows; trigger column carries through.
    for (i, expected) in ["manual", "quick_ask", "focus_change"].iter().enumerate() {
        let row = frames
            .last_in_session(&format!("sess-{i}"))
            .await
            .expect("repo read")
            .expect("frame inserted");
        assert_eq!(row.trigger, *expected, "session {i}");
    }
}

fn sample_envelope(session_id: String, trigger: Trigger) -> SnapshotEnvelope {
    SnapshotEnvelope {
        v: ENVELOPE_VERSION,
        captured_at: Utc::now(),
        device_id: "dev-test".into(),
        session_id,
        frame: FrameSnapshot {
            app: AppInfo {
                bundle_id: Some("com.example.App".into()),
                name: Some("App".into()),
                pid: Some(1),
            },
            window: WindowInfo {
                title: Some("Title".into()),
                is_focused: true,
            },
            url: None,
            screenshot: None,
        },
        extraction: ExtractionResult {
            strategy: ExtractionStrategy::Skipped,
            ax_text: None,
            ocr_text: None,
            adapter_name: None,
            adapter_payload: None,
            selection: None,
            duration_ms: 0,
            fallback_reason: Some("test".into()),
        },
        trigger,
        raw: RawDebug::default(),
    }
}
