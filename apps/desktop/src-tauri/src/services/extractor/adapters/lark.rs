//! Lark / Feishu / Slack adapter — chat workspaces.
//!
//! Lark / Feishu window titles look like
//! `"<channel/dm> · <workspace> · Lark"` (or 飞书 / `<channel>` only on
//! the IM home screen). Slack uses ` | ` separators with the
//! workspace last.
//!
//! ## Two paths under one adapter
//!
//! - **Lark / Feishu** (`com.electron.lark*`, `com.bytedance.feishu*`):
//!   the adapter takes ownership of the AX walk via
//!   `ax_extractor::extract_with_skip` with a skip predicate that
//!   prunes the persistent navigation chrome — window decoration
//!   buttons, the `SideEdgeView` left rail (search box / module tabs
//!   / tenant switcher), the `WatermarkWidget`, and on the IM home
//!   view the `scrollable content` conversation-list container (the
//!   15 navigation tiles for "张博", "Knowledge AI", etc.). Returns
//!   `Extracted`.
//!
//! - **Slack** (`com.tinyspeck.slackmacgap`): we don't have an AX
//!   dump yet so the adapter only parses the window title and returns
//!   `NeedsAxBody` — the dispatcher then runs the unfiltered generic
//!   AX walk for the body.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

const SLACK_BUNDLE: &str = "com.tinyspeck.slackmacgap";

pub struct LarkAdapter;

impl LarkAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LarkAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for LarkAdapter {
    fn name(&self) -> &'static str {
        "lark"
    }

    fn matches(&self, bundle_id: &str) -> bool {
        matches!(
            bundle_id,
            "com.electron.lark"
                | "com.electron.lark.lite"
                | "com.bytedance.feishu"
                | "com.bytedance.feishu.lite"
                | SLACK_BUNDLE
        )
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        let raw = ctx.window_title.trim();
        let channel = parse_chat_channel(raw);
        let payload = json!({
            "channel": channel,
            "raw_window_title": raw,
        });

        // Slack — keep on the generic AX path until we have a dump.
        if ctx.bundle_id == SLACK_BUNDLE {
            return Ok(AdapterOutcome::NeedsAxBody { payload });
        }

        // Lark / Feishu — take over the AX walk with a static skip
        // predicate that prunes navigation chrome. Phase 6 migrated
        // ax_extractor to talk to the helper sidecar over IPC, which
        // can't run arbitrary Rust closures — so the predicate is now
        // a flat set of strings the helper applies its end. The
        // home-view conditional becomes "if home view, also include
        // 'scrollable content' in the skip list".
        #[cfg(target_os = "macos")]
        {
            use super::super::ax_extractor::SkipPredicate;
            let in_home_view = is_lark_home_view(raw);
            let mut skip = SkipPredicate {
                skip_subroles: window_decoration_subroles(),
                skip_descriptions_substr: lark_chrome_descriptions(),
                ..Default::default()
            };
            if in_home_view {
                skip.skip_descriptions_substr
                    .push("scrollable content".into());
            }

            let text = super::super::ax_extractor::extract_with_skip(ctx.pid, skip)
                .await
                .unwrap_or_default();

            return Ok(AdapterOutcome::Extracted { text, payload });
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(AdapterOutcome::NeedsAxBody { payload })
        }
    }
}

// Skip-predicate helpers feed the SkipPredicate constructed inside the
// cfg(macos) branch of `extract()`. On Windows the adapter falls through
// to `NeedsAxBody` without ever building one, so the helpers are dead
// there.
#[cfg(target_os = "macos")]
fn window_decoration_subroles() -> Vec<String> {
    vec![
        "AXCloseButton".into(),
        "AXFullScreenButton".into(),
        "AXMinimizeButton".into(),
    ]
}

#[cfg(target_os = "macos")]
fn lark_chrome_descriptions() -> Vec<String> {
    vec!["SideEdgeView".into(), "WatermarkWidget".into()]
}

/// IM "home" view heuristic: the window title is just `"Lark"` /
/// `"Feishu"` / `"飞书"` with no conversation prefix. Once a
/// conversation is open the title becomes
/// `"<channel> · <workspace> · Lark"`. We treat the absence of
/// `" · "` / `" | "` as the home-view tell. Imperfect but safe — the
/// false-positive case (a conversation whose title parses without a
/// separator) just means we lose its scrollable content for that
/// frame, not the whole capture.
#[cfg(target_os = "macos")]
fn is_lark_home_view(raw_title: &str) -> bool {
    !raw_title.contains(" · ") && !raw_title.contains(" | ")
}

fn parse_chat_channel(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    // Try " · " (Lark / Feishu) then " | " (Slack). First segment is
    // the conversation name in both layouts.
    for sep in [" · ", " | "] {
        if let Some(idx) = raw.find(sep) {
            let head = raw[..idx].trim();
            if !head.is_empty() {
                return Some(head.to_string());
            }
        }
    }
    // No separator → surface whole title; the conversation is the
    // single thing in the window.
    Some(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snapshot_envelope::Trigger;

    fn ctx<'a>(bundle: &'a str, t: &'a str) -> AdapterContext<'a> {
        AdapterContext {
            bundle_id: bundle,
            app_name: "",
            window_title: t,
            pid: None,
            trigger: Trigger::FocusChange,
        }
    }

    #[test]
    fn matches_lark_feishu_and_slack() {
        let a = LarkAdapter::new();
        assert!(a.matches("com.electron.lark"));
        assert!(a.matches("com.bytedance.feishu"));
        assert!(a.matches("com.tinyspeck.slackmacgap"));
        assert!(!a.matches("com.apple.MobileSMS"));
    }

    #[test]
    fn parses_lark_separator() {
        assert_eq!(
            parse_chat_channel("#engineering · Acme · Lark"),
            Some("#engineering".to_string())
        );
    }

    #[test]
    fn parses_slack_separator() {
        assert_eq!(
            parse_chat_channel("#general | Acme Workspace | Slack"),
            Some("#general".to_string())
        );
    }

    #[test]
    fn falls_back_to_whole_title() {
        assert_eq!(
            parse_chat_channel("Direct messages"),
            Some("Direct messages".to_string())
        );
    }

    #[test]
    fn empty_returns_none() {
        assert!(parse_chat_channel("").is_none());
    }

    // The three helpers below (is_lark_home_view / window_decoration_subroles
    // / lark_chrome_descriptions) are `#[cfg(target_os = "macos")]` because
    // they only feed the SkipPredicate built inside the macOS branch of
    // `extract()`. Mirror the same gate on these tests so `cargo test --lib`
    // on Windows / Linux doesn't trip over references to symbols that the
    // compiler strips.
    #[cfg(target_os = "macos")]
    #[test]
    fn home_view_detection() {
        // No separator → IM home view.
        assert!(is_lark_home_view("Feishu"));
        assert!(is_lark_home_view("Lark"));
        assert!(is_lark_home_view("飞书"));
        // Conversation title → not home view.
        assert!(!is_lark_home_view("#engineering · Acme · Lark"));
        assert!(!is_lark_home_view("#general | Acme Workspace | Slack"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn window_decoration_subroles_match_expected() {
        let subroles = window_decoration_subroles();
        assert!(subroles.iter().any(|s| s == "AXCloseButton"));
        assert!(subroles.iter().any(|s| s == "AXFullScreenButton"));
        assert!(subroles.iter().any(|s| s == "AXMinimizeButton"));
        assert!(!subroles.iter().any(|s| s == "AXLandmarkRegion"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lark_chrome_descriptions_match_expected() {
        let descs = lark_chrome_descriptions();
        assert!(descs.iter().any(|s| s == "SideEdgeView"));
        assert!(descs.iter().any(|s| s == "WatermarkWidget"));
        // "scrollable content" is added conditionally based on home-view
        // detection, not part of the static set.
        assert!(!descs.iter().any(|s| s == "scrollable content"));
    }

    #[tokio::test]
    async fn slack_keeps_needs_ax_body() {
        // Slack bundle short-circuits to NeedsAxBody before any AX
        // call — we don't have a dump for it yet.
        let a = LarkAdapter::new();
        let out = a
            .extract(&ctx(
                "com.tinyspeck.slackmacgap",
                "#general | Acme Workspace | Slack",
            ))
            .await
            .expect("ok");
        match out {
            AdapterOutcome::NeedsAxBody { payload } => {
                assert_eq!(payload["channel"], "#general");
            }
            _ => panic!("expected NeedsAxBody for Slack, got {out:?}"),
        }
    }
}
