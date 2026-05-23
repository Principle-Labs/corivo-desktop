//! Per-app adapter framework (spec §六 Phase 5).
//!
//! `FocusAdapter` is the trait every per-app extractor implements. The
//! adapter's `extract` returns an [`AdapterOutcome`] that tells the
//! dispatcher exactly what to do next — short-circuit with the
//! adapter's own text, run the generic AX walk for body, or skip AX
//! entirely and go straight to OCR. The dispatcher itself is pure
//! routing on the outcome enum; it no longer hard-codes confidence
//! levels or per-bundle blacklists.
//!
//! Trait is `async` so adapters that need to walk an AX subtree (which
//! must run on `spawn_blocking` with a wall-clock budget) can do so
//! without each adapter re-implementing the `block_in_place` /
//! `block_on` boilerplate. Today only `GenericAxAdapter` actually
//! awaits — the per-app title-parsers are sync inside but the trait
//! gives them room to grow into deep AX extractors.

pub mod browser;
pub mod generic_ax;
pub mod iterm2;
pub mod jetbrains;
pub mod lark;
pub mod ocr_only;
pub mod registry;
pub mod vscode;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use super::error::Result;
use crate::domain::snapshot_envelope::Trigger;

pub use browser::BrowserAdapter;
pub use generic_ax::GenericAxAdapter;
pub use iterm2::ITerm2Adapter;
pub use jetbrains::JetBrainsAdapter;
pub use lark::LarkAdapter;
pub use ocr_only::OcrOnlyAdapter;
pub use registry::AdapterRegistry;
pub use vscode::VsCodeAdapter;

/// What the adapter knows when it's invoked. `pid` and `bundle_id`
/// come from `foreground::probe`; `window_title` is whatever the
/// frontmost-app metadata reported. `trigger` lets adapters short-
/// circuit on Quick Ask (skip dedup-friendly heuristics like "wait for
/// AX to settle" because the user is waiting for a response right
/// now).
#[derive(Debug, Clone)]
pub struct AdapterContext<'a> {
    pub bundle_id: &'a str,
    pub app_name: &'a str,
    pub window_title: &'a str,
    pub pid: Option<i32>,
    pub trigger: Trigger,
}

/// Tells the dispatcher what to do with the adapter's output. Each
/// variant carries the adapter's own contribution; the dispatcher
/// decides whether to short-circuit, merge with the generic AX walk,
/// or fall through to OCR.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdapterOutcome {
    /// Adapter has produced the canonical extraction. Dispatcher
    /// stores `text` as `frames.ax_text` and `payload` as
    /// `frames.adapter_payload`, then stops.
    Extracted { text: String, payload: Value },
    /// Adapter has a structured side-channel (URL / file path /
    /// channel) but no body text. Dispatcher runs the generic AX walk
    /// for the body and keeps this payload alongside.
    NeedsAxBody { payload: Value },
    /// Adapter knows AX is useless for this app (Figma WebGL canvas,
    /// Warp terminal, Photoshop). Dispatcher skips AX and runs OCR
    /// directly, keeping this payload.
    NeedsOcr { payload: Value, reason: String },
}

/// One adapter per supported app + a generic catch-all. Trait stays
/// dyn-compatible (via `#[async_trait]`) so the registry can hold
/// `Arc<dyn FocusAdapter>`.
#[async_trait]
pub trait FocusAdapter: Send + Sync {
    /// Stable name written into `frames.adapter_name` ('chrome',
    /// 'vscode', 'lark', 'ocr_only', 'generic_ax', ...). Lowercase +
    /// snake.
    fn name(&self) -> &'static str;

    /// Whether this adapter handles `bundle_id`. The registry consults
    /// `matches` in registration order; first hit wins.
    /// `GenericAxAdapter` returns `true` unconditionally and must be
    /// registered last.
    fn matches(&self, bundle_id: &str) -> bool;

    /// Run extraction. Failures here propagate as `Err` and surface in
    /// the dispatcher as `ExtractionStrategy::Skipped` for that frame.
    /// Adapters that simply have nothing useful to contribute should
    /// return `Ok(NeedsAxBody { payload: json!({}) })` instead.
    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome>;
}
