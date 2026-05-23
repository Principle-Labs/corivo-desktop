//! Browser adapter — one table-driven page-title extractor for every
//! supported browser. Each row pairs a bundle id with an optional
//! suffix that the browser appends to its window title (Chromium-family
//! browsers append " - <Brand>"; Safari / Arc keep the title clean).
//!
//! Adding a browser is a single `BrowserSpec` entry — no new file, no
//! registry update. The dispatcher always uses `NeedsAxBody`, so the
//! generic AX walk still produces the body text; we only own the page
//! title parsing.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

struct BrowserSpec {
    bundle_ids: &'static [&'static str],
    /// Suffix the browser appends to its window title. `None` means the
    /// title is already the page title (Safari, Arc). When set, we
    /// strip-suffix and emit the result; if the suffix isn't present
    /// (rare — empty / placeholder titles), `page_title` is null and
    /// the frontend falls back to `focus.window_title`.
    suffix: Option<&'static str>,
}

const BROWSERS: &[BrowserSpec] = &[
    // Chromium family — all share Chrome's " - Google Chrome" tail
    // (Canary / Beta / Dev included; same Info.plist title block).
    BrowserSpec {
        bundle_ids: &[
            "com.google.Chrome",
            "com.google.Chrome.canary",
            "com.google.Chrome.beta",
            "com.google.Chrome.dev",
        ],
        suffix: Some(" - Google Chrome"),
    },
    // Microsoft Edge — release + insider channels.
    BrowserSpec {
        bundle_ids: &[
            "com.microsoft.edgemac",
            "com.microsoft.edgemac.Beta",
            "com.microsoft.edgemac.Dev",
            "com.microsoft.edgemac.Canary",
        ],
        suffix: Some(" - Microsoft Edge"),
    },
    // Brave.
    BrowserSpec {
        bundle_ids: &[
            "com.brave.Browser",
            "com.brave.Browser.beta",
            "com.brave.Browser.nightly",
        ],
        suffix: Some(" - Brave"),
    },
    // Firefox uses an em-dash, not a hyphen.
    BrowserSpec {
        bundle_ids: &[
            "org.mozilla.firefox",
            "org.mozilla.firefoxdeveloperedition",
            "org.mozilla.nightly",
        ],
        suffix: Some(" \u{2014} Mozilla Firefox"),
    },
    // No-suffix browsers: window title IS the page title.
    BrowserSpec {
        bundle_ids: &["com.apple.Safari", "com.apple.SafariTechnologyPreview"],
        suffix: None,
    },
    BrowserSpec {
        bundle_ids: &["company.thebrowser.Browser"],
        suffix: None,
    },
];

fn spec_for(bundle_id: &str) -> Option<&'static BrowserSpec> {
    BROWSERS.iter().find(|b| b.bundle_ids.contains(&bundle_id))
}

pub struct BrowserAdapter;

impl BrowserAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for BrowserAdapter {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        spec_for(bundle_id).is_some()
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        let raw = ctx.window_title.trim();
        let page_title: Option<String> = if raw.is_empty() {
            None
        } else if let Some(suffix) = spec_for(ctx.bundle_id).and_then(|s| s.suffix) {
            // Strict strip — if the suffix isn't there (placeholder
            // titles like "Untitled"), emit null and let the frontend
            // fall back to raw window_title rather than fabricate a
            // page name.
            raw.strip_suffix(suffix)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        } else {
            Some(raw.to_string())
        };
        Ok(AdapterOutcome::NeedsAxBody {
            payload: json!({
                "page_title": page_title,
                "raw_window_title": raw,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(bundle: &'a str, title: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: bundle,
            app_name: "",
            window_title: title,
            pid: Some(1),
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_known_browsers() {
        let a = BrowserAdapter::new();
        assert!(a.matches("com.google.Chrome"));
        assert!(a.matches("com.google.Chrome.canary"));
        assert!(a.matches("com.apple.Safari"));
        assert!(a.matches("company.thebrowser.Browser"));
        assert!(a.matches("org.mozilla.firefox"));
        assert!(a.matches("com.microsoft.edgemac"));
        assert!(a.matches("com.brave.Browser"));
    }

    #[test]
    fn rejects_non_browsers() {
        let a = BrowserAdapter::new();
        assert!(!a.matches("com.microsoft.VSCode"));
        assert!(!a.matches("com.tinyspeck.slackmacgap"));
    }

    #[tokio::test]
    async fn chrome_strips_suffix() {
        let a = BrowserAdapter::new();
        let out = a
            .extract(&ctx("com.google.Chrome", "rust-lang.org - Google Chrome"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["page_title"], "rust-lang.org");
                assert_eq!(payload["raw_window_title"], "rust-lang.org - Google Chrome");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }

    #[tokio::test]
    async fn chrome_without_suffix_yields_null_page_title() {
        let a = BrowserAdapter::new();
        let out = a
            .extract(&ctx("com.google.Chrome", "Untitled"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert!(payload["page_title"].is_null());
                assert_eq!(payload["raw_window_title"], "Untitled");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }

    #[tokio::test]
    async fn safari_passes_through_raw() {
        let a = BrowserAdapter::new();
        let out = a
            .extract(&ctx("com.apple.Safari", "Hacker News"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["page_title"], "Hacker News");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }

    #[tokio::test]
    async fn arc_passes_through_raw() {
        let a = BrowserAdapter::new();
        let out = a
            .extract(&ctx("company.thebrowser.Browser", "Linear"))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["page_title"], "Linear");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }

    #[tokio::test]
    async fn firefox_strips_em_dash_suffix() {
        let a = BrowserAdapter::new();
        let out = a
            .extract(&ctx(
                "org.mozilla.firefox",
                "Hacker News \u{2014} Mozilla Firefox",
            ))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["page_title"], "Hacker News");
            }
            _ => panic!("expected NeedsAxBody, got {out:?}"),
        }
    }
}
