//! Local consumer — writes one `frames` row per envelope (spec §四 / §五).
//!
//! Phase 1 is the only consumer in production. Cloud / dual-write variants
//! land in Phase 6+ via the same `SnapshotConsumer` trait.

use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    db::repos::frames::FrameRepo,
    domain::{
        focus_context::FocusContext,
        frame::NewFrame,
        snapshot_envelope::{ExtractionStrategy, SnapshotEnvelope},
    },
    error::Result,
    services::tokenize::tokenize_for_index,
};

use super::SnapshotConsumer;

/// Event name the Quick Ask overlay listens to so it can re-aim its
/// FocusCard at the user's current foreground app the instant the
/// capture pipeline notices a focus change. Payload is a [`FocusContext`].
pub const FRAME_INGESTED_EVENT: &str = "capture:frame-ingested";

/// Webview label for the Quick Ask overlay. We `emit_to` it
/// explicitly because broadcast `emit` has been observed to miss the
/// separate panel webview in some Tauri 2 setups.
const QUICK_ASK_LABEL: &str = "quick-ask";

pub struct LocalConsumer {
    frames: Arc<dyn FrameRepo>,
    /// `None` in unit tests; production sets this so the Quick Ask
    /// overlay can auto-follow focus changes without a manual refresh.
    app: Option<AppHandle>,
    /// Tauri bundle identifier of *this* process. Pulled from
    /// `app.config().identifier` at construction so dev (`…desktop.dev`)
    /// and prod (`…desktop`) builds each carry the right value without
    /// hard-coding. Used to suppress the Quick Ask emit when the user
    /// clicks into the Quick Ask panel itself — capture still ingests
    /// the frame, but the overlay should keep aiming at the user's
    /// previous app, not the chrome they're typing into.
    self_bundle_id: Option<String>,
}

impl LocalConsumer {
    pub fn new(frames: Arc<dyn FrameRepo>) -> Self {
        Self {
            frames,
            app: None,
            self_bundle_id: None,
        }
    }

    pub fn with_app(frames: Arc<dyn FrameRepo>, app: AppHandle) -> Self {
        let self_bundle_id = Some(app.config().identifier.clone());
        Self {
            frames,
            app: Some(app),
            self_bundle_id,
        }
    }
}

#[async_trait]
impl SnapshotConsumer for LocalConsumer {
    async fn ingest(&self, env: SnapshotEnvelope) -> Result<()> {
        let new = envelope_to_new_frame(&env);
        let frame = self.frames.insert(new).await?;
        tracing::info!(
            frame_id = %frame.id,
            captured_at = %frame.captured_at,
            strategy = frame.extraction_strategy.as_str(),
            "snapshot_consumer.local.ingested"
        );

        if let Some(app) = &self.app {
            // Skip the overlay emit when the just-ingested frame is
            // Corivo itself — otherwise clicking into the Quick Ask
            // panel would re-aim its FocusCard at us. The frame still
            // lands in the `frames` table; only the overlay event is
            // suppressed. Mirrors the self-skip the NSWorkspace observer
            // applies to `capture:focus-activated`. Two checks because
            // the helper sometimes hands us a bundle id that doesn't
            // exactly equal `app.config().identifier` (e.g. unsigned
            // dev builds where NSRunningApplication falls back to the
            // bare prefix), so the prefix engine rule covers those
            // corners.
            let bundle_id = env.frame.app.bundle_id.as_deref();
            let is_exact_self = bundle_id.is_some() && bundle_id == self.self_bundle_id.as_deref();
            let is_prefix_self = bundle_id
                .map(crate::services::exclusion::matches_self)
                .unwrap_or(false);
            if is_exact_self || is_prefix_self {
                tracing::debug!(
                    frame_id = %frame.id,
                    app_bundle_id = ?env.frame.app.bundle_id,
                    "snapshot_consumer.local.emit_skipped_self"
                );
            } else {
                // Build the FocusContext directly from the live envelope
                // (with the just-assigned frame_id) instead of from the
                // persisted Frame row. The DB schema doesn't carry every
                // ExtractionResult field — selection, in particular, is
                // intentionally not persisted (it's a transient user
                // signal, not part of the captured frame). Routing
                // through the envelope avoids a lossy round-trip and
                // keeps the auto-update path's FocusContext shape-equal
                // to the hotkey path's.
                let payload = envelope_to_focus_context(&env, &frame.id);
                // Same `emit_to(&str)` ↔ `listen()` target mismatch the
                // nsworkspace observer hit: route the event through the
                // overlay's webview window directly so it lands.
                match app.get_webview_window(QUICK_ASK_LABEL) {
                    Some(window) => match window.emit(FRAME_INGESTED_EVENT, &payload) {
                        Ok(_) => tracing::info!(
                            frame_id = %frame.id,
                            app_bundle_id = ?env.frame.app.bundle_id,
                            target = QUICK_ASK_LABEL,
                            "snapshot_consumer.local.emit_ok"
                        ),
                        Err(error) => tracing::warn!(
                            ?error,
                            frame_id = %frame.id,
                            "snapshot_consumer.local.emit_failed"
                        ),
                    },
                    None => tracing::warn!(
                        target = QUICK_ASK_LABEL,
                        "snapshot_consumer.local.quick_ask_window_not_found"
                    ),
                }
            }
        } else {
            tracing::warn!(
                "snapshot_consumer.local.no_app_handle — built via ::new instead of ::with_app; \
                 Quick Ask will NOT auto-update"
            );
        }

        Ok(())
    }
}

/// Build a [`FocusContext`] from the in-memory [`SnapshotEnvelope`]
/// plus the frame id we just got back from the repo. Mirrors the
/// hotkey-path construction in
/// `capture_pipeline::invoke_quick_ask_phase_b` so the auto-update
/// path produces the same shape — including transient fields like
/// `selection` that aren't carried by the persisted `frames` schema.
///
/// This intentionally does NOT read from the persisted `Frame` row.
/// Going through the DB would force every value the LLM eventually
/// sees to also be a column, which conflates "what we want claude to
/// know about the user's current state" with "what we want to query
/// historically". The envelope is the right primitive.
fn envelope_to_focus_context(env: &SnapshotEnvelope, frame_id: &str) -> FocusContext {
    let primary_text = env
        .extraction
        .ax_text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| env.extraction.ocr_text.clone())
        .unwrap_or_default();
    let empty = primary_text.trim().is_empty();

    FocusContext {
        frame_id: frame_id.to_string(),
        captured_at: env.captured_at,
        app_bundle_id: env.frame.app.bundle_id.clone(),
        app_name: env.frame.app.name.clone(),
        window_title: env.frame.window.title.clone(),
        url: env.frame.url.clone(),
        adapter_name: env.extraction.adapter_name.clone(),
        adapter_payload: env.extraction.adapter_payload.clone(),
        primary_text,
        selection: env.extraction.selection.clone(),
        excluded: env.raw.exclusion_match.is_some(),
        empty,
    }
}

/// Flatten a `SnapshotEnvelope` into the row shape the `frames` table
/// expects. Pure function — no I/O — so it can be unit-tested in isolation.
fn envelope_to_new_frame(env: &SnapshotEnvelope) -> NewFrame {
    let strategy = env.extraction.strategy;
    let ax_text = env.extraction.ax_text.clone();
    let ocr_text = env.extraction.ocr_text.clone();
    let adapter_payload_str = env
        .extraction
        .adapter_payload
        .as_ref()
        .map(|v| v.to_string());

    let search_tokens = build_search_tokens(env, &ax_text, &ocr_text);

    NewFrame {
        captured_at: env.captured_at,
        device_id: env.device_id.clone(),
        capture_session_id: env.session_id.clone(),

        app_bundle_id: env.frame.app.bundle_id.clone(),
        app_name: env.frame.app.name.clone(),
        window_title: env.frame.window.title.clone(),
        url: env.frame.url.clone(),

        screenshot_path: env.frame.screenshot.as_ref().map(|s| s.path.clone()),
        screenshot_hash: env.frame.screenshot.as_ref().map(|s| s.hash.clone()),
        screenshot_size_bytes: env.frame.screenshot.as_ref().and_then(|s| s.size_bytes),

        ax_text: ax_text.clone(),
        ocr_text: ocr_text.clone(),
        // v1600: classify-at-capture (Hook A) 已撤销 —— PII spans 不再
        // 落盘。隐私过滤改在 exec_agent.rs 出口处 (Hook B) 现算现用。
        adapter_name: env.extraction.adapter_name.clone(),
        adapter_payload: adapter_payload_str.clone(),
        extraction_strategy: strategy.as_str().to_string(),
        extraction_duration_ms: Some(env.extraction.duration_ms as i64),
        fallback_reason: env.extraction.fallback_reason.clone(),

        trigger: env.trigger.as_str().to_string(),
        content_hash: compute_content_hash(
            strategy,
            ax_text.as_deref(),
            ocr_text.as_deref(),
            adapter_payload_str.as_deref(),
        ),
        exclusion_match: env.raw.exclusion_match.clone(),
        search_tokens,
    }
}

/// Compose the FTS5 input from every text-bearing field on the
/// envelope, then segment via jieba so CJK queries match through the
/// unicode61 tokenizer (which would otherwise see Chinese as one
/// giant token).
fn build_search_tokens(
    env: &SnapshotEnvelope,
    ax_text: &Option<String>,
    ocr_text: &Option<String>,
) -> Option<String> {
    let mut buf = String::new();
    let mut push = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            return;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(t);
    };
    if let Some(name) = env.frame.app.name.as_deref() {
        push(name);
    }
    if let Some(title) = env.frame.window.title.as_deref() {
        push(title);
    }
    if let Some(url) = env.frame.url.as_deref() {
        push(url);
    }
    if let Some(text) = ax_text.as_deref() {
        push(text);
    }
    if let Some(text) = ocr_text.as_deref() {
        push(text);
    }
    if buf.is_empty() {
        return None;
    }
    let tokens = tokenize_for_index(&buf);
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

/// Compute the dedup hash per spec §五:
/// `SHA256(ax_text || '|' || ocr_text || '|' || adapter_payload)`.
///
/// Returns `None` for `Skipped` strategy (no extracted text → no
/// meaningful hash) so dedup logic in capture_pipeline can rely on
/// `Some` <=> "we have real text to compare".
fn compute_content_hash(
    strategy: ExtractionStrategy,
    ax_text: Option<&str>,
    ocr_text: Option<&str>,
    adapter_payload: Option<&str>,
) -> Option<String> {
    if strategy == ExtractionStrategy::Skipped {
        return None;
    }
    let ax = ax_text.unwrap_or("");
    let ocr = ocr_text.unwrap_or("");
    let payload = adapter_payload.unwrap_or("");
    if ax.is_empty() && ocr.is_empty() && payload.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(ax.as_bytes());
    hasher.update(b"|");
    hasher.update(ocr.as_bytes());
    hasher.update(b"|");
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    Some(format!("sha256:{}", hex::encode(digest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::{
        AppInfo, ExtractionResult, FrameSnapshot, RawDebug, ScreenshotRef, Trigger, WindowInfo,
        ENVELOPE_VERSION,
    };
    use chrono::{TimeZone, Utc};

    fn sample_env() -> SnapshotEnvelope {
        SnapshotEnvelope {
            v: ENVELOPE_VERSION,
            captured_at: Utc.with_ymd_and_hms(2026, 4, 27, 10, 14, 23).unwrap(),
            device_id: "dev".into(),
            session_id: "sess".into(),
            frame: FrameSnapshot {
                app: AppInfo {
                    bundle_id: Some("com.example".into()),
                    name: Some("Example".into()),
                    pid: Some(1),
                },
                window: WindowInfo {
                    title: Some("hi".into()),
                    is_focused: true,
                },
                url: None,
                screenshot: Some(ScreenshotRef {
                    path: "captures/x.jpg".into(),
                    hash: "sha256:image".into(),
                    size_bytes: Some(100),
                }),
            },
            extraction: ExtractionResult {
                strategy: ExtractionStrategy::Ocr,
                ax_text: None,
                ocr_text: Some("hello".into()),
                adapter_name: None,
                adapter_payload: None,
                selection: None,
                duration_ms: 50,
                fallback_reason: None,
            },
            trigger: Trigger::FocusChange,
            raw: RawDebug::default(),
        }
    }

    #[test]
    fn envelope_to_new_frame_flattens_correctly() {
        let env = sample_env();
        let new = envelope_to_new_frame(&env);
        assert_eq!(new.app_bundle_id.as_deref(), Some("com.example"));
        assert_eq!(new.window_title.as_deref(), Some("hi"));
        assert_eq!(new.screenshot_path.as_deref(), Some("captures/x.jpg"));
        assert_eq!(new.extraction_strategy, "ocr");
        assert_eq!(new.ocr_text.as_deref(), Some("hello"));
        assert!(new.content_hash.is_some());
    }

    #[test]
    fn skipped_strategy_has_no_content_hash() {
        let mut env = sample_env();
        env.extraction.strategy = ExtractionStrategy::Skipped;
        env.extraction.ocr_text = None;
        env.raw.exclusion_match = Some("app:foo".into());
        let new = envelope_to_new_frame(&env);
        assert_eq!(new.extraction_strategy, "skipped");
        assert!(new.content_hash.is_none());
        assert_eq!(new.exclusion_match.as_deref(), Some("app:foo"));
    }

    #[test]
    fn content_hash_changes_with_text() {
        let h1 = compute_content_hash(ExtractionStrategy::Ocr, None, Some("hello"), None).unwrap();
        let h2 = compute_content_hash(ExtractionStrategy::Ocr, None, Some("world"), None).unwrap();
        assert_ne!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn ax_and_ocr_combined_via_pipe() {
        // The pipe separator means "ax|ocr" hashes differently from
        // "axocr" with empty fields — a regression guard for the hash
        // formula in spec §五.
        let h_split =
            compute_content_hash(ExtractionStrategy::AxOcr, Some("a"), Some("b"), None).unwrap();
        let h_glued =
            compute_content_hash(ExtractionStrategy::AxOcr, Some("ab"), Some(""), None).unwrap();
        assert_ne!(h_split, h_glued);
    }

    #[test]
    fn adapter_payload_participates_in_content_hash() {
        let base =
            compute_content_hash(ExtractionStrategy::Adapter, Some("text"), None, Some("{}"))
                .unwrap();
        let with_payload = compute_content_hash(
            ExtractionStrategy::Adapter,
            Some("text"),
            None,
            Some(r#"{"url":"https://example.com"}"#),
        )
        .unwrap();
        assert_ne!(base, with_payload);
    }
}
