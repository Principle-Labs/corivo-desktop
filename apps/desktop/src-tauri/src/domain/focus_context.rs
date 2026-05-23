//! Quick Ask focus payload (spec §六 + §八).
//!
//! Returned by `capture_pipeline::invoke_quick_ask()` and surfaced
//! to the floating window so it can render the "正在阅读 / 正在编辑 /
//! 在 #channel 看消息" header card. Same shape goes into the recall
//! orchestrator's initial user message as the `context` part.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FocusContext {
    /// ULID of the frame the invoke wrote. The recall orchestrator
    /// uses this to attach `cited_frame_ids` automatically and to let
    /// the LLM call `get_frame_text(frame_id)` if it wants more detail.
    pub frame_id: String,
    pub captured_at: DateTime<Utc>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    /// `chrome` / `vscode` / `lark` / `generic_ax` / etc. Lets the UI
    /// pick the right icon + verb ("阅读" vs "编辑" vs "在群里").
    pub adapter_name: Option<String>,
    /// Adapter-specific structured fields (URL / file path / cwd /
    /// channel / selection ...). Opaque to the UI except the FocusCard
    /// optionally pulls a couple of well-known keys for display.
    pub adapter_payload: Option<serde_json::Value>,
    /// Extracted text the orchestrator passes inline to the LLM. The
    /// floating window shows the first ~200 chars; full body is
    /// available via `get_frame_text(frame_id)`.
    pub primary_text: String,
    /// User's currently-highlighted text, captured via the AX
    /// `AXSelectedTextMarkerRange` post-step (see
    /// [`crate::services::extractor::selection_probe`]). `None` when
    /// there's no active selection or the host app doesn't expose
    /// marker ranges. Quick Ask surfaces this to the LLM as a
    /// separate "user highlighted" block alongside `primary_text` so
    /// the model can distinguish "what's on screen" from "what the
    /// user is asking about specifically".
    #[serde(default)]
    pub selection: Option<String>,
    /// True when the focus app hit the exclusion engine — Quick Ask
    /// then displays "该应用已被排除" and `primary_text` is empty.
    pub excluded: bool,
    /// True when AX permission isn't granted or every adapter returned
    /// nothing. UI surfaces "无法读取窗口内容,可继续无 context 提问".
    pub empty: bool,
}

impl FocusContext {
    /// Short label for the FocusCard. UI picks emoji prefix off this.
    pub fn human_summary(&self) -> String {
        if self.excluded {
            return format!(
                "🔒 {} 已被排除",
                self.app_name.as_deref().unwrap_or("当前应用")
            );
        }
        if self.empty {
            return "💭 无法读取窗口内容".into();
        }
        let app = self
            .app_name
            .as_deref()
            .or(self.app_bundle_id.as_deref())
            .unwrap_or("当前窗口");
        let detail = self
            .window_title
            .as_deref()
            .or(self.url.as_deref())
            .unwrap_or("");
        if detail.is_empty() {
            format!("📖 在 {app}")
        } else {
            format!("📖 {app} · {detail}")
        }
    }
}

/// Subset of [`FocusContext`] that's available immediately after the
/// fast probe + screenshot phase (before the AX walk + adapter pipeline
/// runs). Sent to the Quick Ask window in the `quick-ask:opened` event
/// so the FocusCard can render the title row right away while the
/// extractor finishes in the background.
///
/// The full [`FocusContext`] arrives later via `quick-ask:focus-ready`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickAskSkeleton {
    pub captured_at: DateTime<Utc>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
}
