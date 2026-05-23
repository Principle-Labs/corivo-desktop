//! Strategy dispatcher for the v3 extractor (spec §六, ax-ocr-spec.md §五).
//!
//! Pure routing on [`AdapterOutcome`]:
//!
//! - `Extracted` (per-app adapter has body) → strategy = `Adapter`.
//! - `Extracted` (generic_ax has body) → strategy = `Ax`.
//! - `NeedsAxBody` → run the generic adapter for body, merge per-app
//!   payload alongside. If generic emits `NeedsOcr`, fall through.
//! - `NeedsOcr` → on-demand screenshot via the helper sidecar + persist
//!   via `CaptureStore` + Vision OCR. **The pipeline never takes a
//!   pre-emptive screenshot anymore**: the only screenshots that ever
//!   land on disk are the ones OCR fallback actually needed.
//!
//! All AX physics (the `ax_extractor::extract` call, the
//! `AX_MIN_CHARS` threshold, the OCR-only bundle blacklist) live
//! inside adapters. The dispatcher itself is a `match` over the enum
//! plus the OCR plumbing.
//!
//! Both macOS and Windows route AX + screen capture + OCR through the
//! capture-helper sidecar (`ScreenCaptureKit` + Vision on macOS; WGC +
//! `Windows.Media.Ocr` on Windows). The dispatcher is now host-agnostic;
//! per-app adapter eligibility is decided by the adapter registry's own
//! `matches()` calls (which on Windows currently boils down to the
//! generic AX adapter, since the bundle-id namespace is macOS-specific).

use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::domain::snapshot_envelope::{ExtractionStrategy, ScreenshotRef, Trigger};
use crate::services::capture_store::CaptureStore;

use super::adapters::generic_ax::NAME as GENERIC_AX_NAME;
use super::adapters::{AdapterContext, AdapterOutcome, AdapterRegistry};
use super::error::Result;

/// Everything the dispatcher needs to take + persist a screenshot for
/// the OCR fallback path. Lives outside [`ExtractInput`] so the AX-only
/// happy path doesn't allocate / clone the `Arc<CaptureStore>` on every
/// tick.
#[derive(Clone)]
pub struct OcrResources {
    pub capture_store: Arc<CaptureStore>,
    pub session_id: String,
    pub jpeg_quality: u8,
}

/// Inputs the dispatcher needs to decide / run an extraction.
#[derive(Clone)]
pub struct ExtractInput {
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub window_pid: Option<i32>,
    pub captured_at: DateTime<Utc>,
    pub trigger: Trigger,
    /// Adapter registry to consult. Borrowed via `Arc` so
    /// capture_pipeline keeps a single shared registry instead of
    /// rebuilding one per tick. Optional so older test paths without
    /// an adapter pipeline still compile — those paths skip extraction.
    pub adapters: Option<Arc<AdapterRegistry>>,
    /// When `Some`, a `NeedsOcr` outcome triggers an on-demand screen
    /// capture + Vision OCR. When `None`, those branches fall through
    /// to `Skipped` (used by unit tests + non-macOS hosts where we
    /// can't usefully take a screenshot anyway).
    pub ocr_resources: Option<OcrResources>,
}

/// What the dispatcher produced for one capture. Maps 1:1 to
/// `ExtractionResult` in `domain::snapshot_envelope` plus an optional
/// `screenshot` the caller can put on `FrameSnapshot.screenshot`.
#[derive(Debug, Clone)]
pub struct ExtractionOutcome {
    pub strategy: ExtractionStrategy,
    pub ax_text: Option<String>,
    pub ocr_text: Option<String>,
    /// Per-app adapter that produced this row. None when the generic
    /// AX path or pure OCR fallback ran without an adapter
    /// contributing.
    pub adapter_name: Option<String>,
    /// Adapter-specific JSON payload (URL / file path / cwd /
    /// channel ...). None when no adapter contributed.
    pub adapter_payload: Option<Value>,
    /// User's currently-highlighted text, captured via the AXTextMarker
    /// post-step (see [`super::selection_probe`]). Independent of the
    /// strategy ladder above — runs whenever we have a pid, regardless
    /// of whether the body came from an adapter, generic AX, or OCR.
    pub selection: Option<String>,
    /// Reference to the on-disk screenshot, if OCR fallback ran and
    /// produced one. `None` for AX-only and adapter rows.
    pub screenshot: Option<ScreenshotRef>,
    pub duration_ms: u32,
    pub fallback_reason: Option<String>,
}

/// Run the active strategy ladder for the current host, then layer
/// the AX-based selection probe on top. The selection probe is a
/// post-step deliberately decoupled from the body-text ladder: it
/// runs even when the body came from OCR (where AX wasn't useful for
/// extraction but still tells us what's selected) and skips only on
/// `Skipped` outcomes (where the app is exclusion-listed and we
/// shouldn't be reading anything).
pub async fn extract(input: &ExtractInput) -> Result<ExtractionOutcome> {
    let started = Instant::now();
    let mut outcome = run_strategy(input, started).await?;
    if outcome.strategy != ExtractionStrategy::Skipped {
        outcome.selection = super::selection_probe::probe(input.window_pid).await;
    }
    Ok(outcome)
}

async fn run_strategy(input: &ExtractInput, started: Instant) -> Result<ExtractionOutcome> {
    let Some(registry) = input.adapters.as_ref() else {
        // No adapter registry wired (legacy / test paths). Treat as
        // "no adapter ran" → try OCR if resources available, else skip.
        return run_ocr_fallback(
            input.ocr_resources.as_ref(),
            started,
            "no_registry".into(),
            None,
            None,
        )
        .await;
    };

    let bundle = input.app_bundle_id.as_deref().unwrap_or("");
    let app_name = input.app_name.as_deref().unwrap_or("");
    let window_title = input.window_title.as_deref().unwrap_or("");
    let ctx = AdapterContext {
        bundle_id: bundle,
        app_name,
        window_title,
        pid: input.window_pid,
        trigger: input.trigger,
    };

    let adapter = registry.find(bundle);
    let adapter_name = adapter.name().to_string();
    let outcome = adapter.extract(&ctx).await?;

    match outcome {
        AdapterOutcome::Extracted { text, payload } => {
            let is_generic = adapter_name == GENERIC_AX_NAME;
            Ok(ExtractionOutcome {
                strategy: if is_generic {
                    ExtractionStrategy::Ax
                } else {
                    ExtractionStrategy::Adapter
                },
                ax_text: Some(text),
                ocr_text: None,
                adapter_name: if is_generic { None } else { Some(adapter_name) },
                adapter_payload: if is_generic { None } else { Some(payload) },
                selection: None,
                screenshot: None,
                duration_ms: started.elapsed().as_millis() as u32,
                fallback_reason: None,
            })
        }

        AdapterOutcome::NeedsAxBody {
            payload: side_payload,
        } => {
            // Per-app adapter only contributed structured fields;
            // delegate to the generic adapter for body text.
            let body_outcome = registry.generic().extract(&ctx).await?;
            match body_outcome {
                AdapterOutcome::Extracted { text, .. } => Ok(ExtractionOutcome {
                    strategy: ExtractionStrategy::Ax,
                    ax_text: Some(text),
                    ocr_text: None,
                    adapter_name: Some(adapter_name),
                    adapter_payload: Some(side_payload),
                    selection: None,
                    screenshot: None,
                    duration_ms: started.elapsed().as_millis() as u32,
                    fallback_reason: None,
                }),
                AdapterOutcome::NeedsOcr { reason, .. } => {
                    run_ocr_fallback(
                        input.ocr_resources.as_ref(),
                        started,
                        reason,
                        Some(adapter_name),
                        Some(side_payload),
                    )
                    .await
                }
                AdapterOutcome::NeedsAxBody { .. } => {
                    // generic_ax never emits NeedsAxBody by contract.
                    Err(super::error::ExtractorError::Internal(
                        "generic_ax returned NeedsAxBody (unreachable)".into(),
                    ))
                }
            }
        }

        AdapterOutcome::NeedsOcr { payload, reason } => {
            let is_generic = adapter_name == GENERIC_AX_NAME;
            run_ocr_fallback(
                input.ocr_resources.as_ref(),
                started,
                reason,
                if is_generic { None } else { Some(adapter_name) },
                if is_generic { None } else { Some(payload) },
            )
            .await
        }
    }
}

/// OCR fallback: take a screenshot via the helper sidecar, persist it
/// through `CaptureStore`, then run Vision OCR. When `ocr_resources`
/// is `None` (test paths, non-mac builds, OCR disabled at the call
/// site), short-circuit to a `Skipped` outcome — no image is taken
/// and `screenshot` stays `None` so the frame ingest doesn't reference
/// a non-existent path.
async fn run_ocr_fallback(
    ocr_resources: Option<&OcrResources>,
    started: Instant,
    reason: String,
    adapter_name: Option<String>,
    adapter_payload: Option<Value>,
) -> Result<ExtractionOutcome> {
    use super::ocr_extractor;
    use crate::services::capture_pipeline::screen_capture;

    let Some(res) = ocr_resources else {
        return Ok(skipped(
            started,
            format!("{reason}|no_ocr_resources"),
            adapter_name,
            adapter_payload,
            None,
        ));
    };

    // 1. Take JPEG via the helper sidecar.
    let capture = screen_capture::take_jpeg(res.jpeg_quality)
        .await
        .map_err(|e| {
            super::error::ExtractorError::Internal(format!("on-demand jpeg failed: {e}"))
        })?;
    let hash = capture.screenshot_hash();
    let size_bytes = capture.jpeg.len() as i64;

    // 2. Persist into the captures store.
    let (abs_path, _) = res
        .capture_store
        .save_screenshot(&res.session_id, capture.jpeg)
        .await
        .map_err(|e| {
            super::error::ExtractorError::Internal(format!("save_screenshot failed: {e}"))
        })?;
    let rel_path = abs_path
        .strip_prefix(res.capture_store.captures_root())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned());
    let screenshot_ref = ScreenshotRef {
        path: rel_path,
        hash,
        size_bytes: Some(size_bytes),
    };

    // 3. OCR.
    let ocr_text = ocr_extractor::extract(&abs_path).await?;
    let duration_ms = started.elapsed().as_millis() as u32;

    if ocr_text.is_empty() {
        Ok(ExtractionOutcome {
            strategy: ExtractionStrategy::Skipped,
            ax_text: None,
            ocr_text: None,
            adapter_name,
            adapter_payload,
            selection: None,
            screenshot: Some(screenshot_ref),
            duration_ms,
            fallback_reason: Some(format!("{reason}|ocr_empty")),
        })
    } else {
        Ok(ExtractionOutcome {
            strategy: ExtractionStrategy::Ocr,
            ax_text: None,
            ocr_text: Some(ocr_text),
            adapter_name,
            adapter_payload,
            selection: None,
            screenshot: Some(screenshot_ref),
            duration_ms,
            fallback_reason: Some(reason),
        })
    }
}

fn skipped(
    started: Instant,
    reason: String,
    adapter_name: Option<String>,
    adapter_payload: Option<Value>,
    screenshot: Option<ScreenshotRef>,
) -> ExtractionOutcome {
    ExtractionOutcome {
        strategy: ExtractionStrategy::Skipped,
        ax_text: None,
        ocr_text: None,
        adapter_name,
        adapter_payload,
        selection: None,
        screenshot,
        duration_ms: started.elapsed().as_millis() as u32,
        fallback_reason: Some(reason),
    }
}
