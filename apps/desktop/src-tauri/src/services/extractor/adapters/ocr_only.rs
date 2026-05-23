//! `OcrOnlyAdapter` — apps whose AX tree is known to be useless
//! (Figma WebGL canvas, GPU-rendered terminals, Photoshop / Illustrator
//! / After Effects). These render their content to a GPU surface, so
//! AX walking returns chrome-only labels. The adapter immediately
//! emits `NeedsOcr` and the dispatcher skips the AX layer.
//!
//! This list is curated, not exhaustive — Phase 2 dogfood will catch
//! the rest.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

const OCR_ONLY_BUNDLES: &[&str] = &[
    // GPU / canvas-rendered design + terminals
    "com.figma.Desktop",
    "dev.warp.Warp-Stable",
    "org.alacritty",
    "net.kovidgoyal.kitty",
    // Adobe — main canvases are GPU-rendered
    "com.adobe.Photoshop",
    "com.adobe.illustrator",
    "com.adobe.AfterEffects",
];

pub struct OcrOnlyAdapter;

impl OcrOnlyAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OcrOnlyAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for OcrOnlyAdapter {
    fn name(&self) -> &'static str {
        "ocr_only"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        !bundle_id.is_empty() && OCR_ONLY_BUNDLES.contains(&bundle_id)
    }

    async fn extract(&self, _ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        Ok(AdapterOutcome::NeedsOcr {
            payload: json!({
                "kind": "ocr_only",
            }),
            reason: "bundle_blacklist".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(bundle: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: bundle,
            app_name: "",
            window_title: "",
            pid: None,
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_known_offenders() {
        let a = OcrOnlyAdapter::new();
        assert!(a.matches("com.figma.Desktop"));
        assert!(a.matches("dev.warp.Warp-Stable"));
        assert!(a.matches("org.alacritty"));
        assert!(a.matches("net.kovidgoyal.kitty"));
        assert!(a.matches("com.adobe.Photoshop"));
    }

    #[test]
    fn does_not_match_empty_or_unknown() {
        let a = OcrOnlyAdapter::new();
        assert!(!a.matches(""));
        assert!(!a.matches("com.google.Chrome"));
    }

    #[tokio::test]
    async fn emits_needs_ocr_with_bundle_blacklist_reason() {
        let a = OcrOnlyAdapter::new();
        let out = a.extract(&ctx("com.figma.Desktop")).await.expect("ok");
        match out {
            AdapterOutcome::NeedsOcr { reason, .. } => {
                assert_eq!(reason, "bundle_blacklist");
            }
            _ => panic!("expected NeedsOcr, got {out:?}"),
        }
    }
}
