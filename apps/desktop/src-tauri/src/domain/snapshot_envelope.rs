//! `SnapshotEnvelope` — the only payload that crosses the capture →
//! storage seam (spec §四).
//!
//! Capture pipeline produces one of these per capture; a `SnapshotConsumer`
//! consumes it. The envelope is versioned so future cloud uploaders can
//! still deserialize older client builds.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const ENVELOPE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEnvelope {
    pub v: u32,
    pub captured_at: DateTime<Utc>,
    pub device_id: String,
    pub session_id: String,
    pub frame: FrameSnapshot,
    pub extraction: ExtractionResult,
    /// What woke capture_pipeline up. The pipeline is event-driven —
    /// every variant traces back to a concrete OS / IPC / observer
    /// event. `QuickAsk` is invoked synchronously from the global
    /// hotkey; everything else flows through the event bus.
    ///
    /// Default for missing-field deserialization is `Manual` —
    /// shouldn't happen on real envelopes (capture_pipeline always
    /// stamps it explicitly), but a defensive fallback is cheap.
    #[serde(default = "Trigger::default_for_serde")]
    pub trigger: Trigger,
    #[serde(default)]
    pub raw: RawDebug,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// NSWorkspace observer: frontmost app changed.
    FocusChange,
    /// AXObserver: the focused window inside the current app changed
    /// (kAXFocusedWindowChanged).
    FocusedWindowChanged,
    /// AXObserver: window title changed (kAXTitleChanged) — the main
    /// signal we use to detect URL / page changes inside browsers and
    /// SPA-style apps.
    TitleChanged,
    /// Quick Ask global-hotkey invoke.
    QuickAsk,
    /// Tauri IPC requested an immediate snapshot
    /// (`capture_request_snapshot`).
    Manual,
}

impl Trigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FocusChange => "focus_change",
            Self::FocusedWindowChanged => "focused_window_changed",
            Self::TitleChanged => "title_changed",
            Self::QuickAsk => "quick_ask",
            Self::Manual => "manual",
        }
    }

    fn default_for_serde() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameSnapshot {
    pub app: AppInfo,
    pub window: WindowInfo,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub screenshot: Option<ScreenshotRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppInfo {
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub pid: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowInfo {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub is_focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenshotRef {
    pub path: String,
    pub hash: String,
    #[serde(default)]
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractionResult {
    pub strategy: ExtractionStrategy,
    #[serde(default)]
    pub ax_text: Option<String>,
    #[serde(default)]
    pub ocr_text: Option<String>,
    /// Phase 5: which per-app adapter produced this output. `None`
    /// when extraction didn't go through the adapter framework
    /// (Phase 1-4 rows / OCR-only / skipped).
    #[serde(default)]
    pub adapter_name: Option<String>,
    /// Phase 5: adapter-specific structured fields (URL / file path /
    /// cwd / channel / ...). Stored as JSON so it crosses the adapter
    /// boundary opaquely.
    #[serde(default)]
    pub adapter_payload: Option<serde_json::Value>,
    /// User's currently-highlighted text, captured via AXTextMarker
    /// (see [`crate::services::extractor::selection_probe`]).
    /// Populated independently of the adapter pipeline — works in any
    /// app that implements the AX text-marker family (browsers,
    /// TextEdit, Notes, Pages, etc.). `None` when there's no
    /// selection or the host app doesn't expose marker ranges.
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(default)]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExtractionStrategy {
    #[serde(rename = "ax")]
    Ax,
    #[serde(rename = "ocr")]
    Ocr,
    #[serde(rename = "ax+ocr")]
    AxOcr,
    /// Phase 5: per-app adapter produced the row.
    #[serde(rename = "adapter")]
    Adapter,
    #[serde(rename = "skipped")]
    Skipped,
}

impl ExtractionStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ax => "ax",
            Self::Ocr => "ocr",
            Self::AxOcr => "ax+ocr",
            Self::Adapter => "adapter",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawDebug {
    #[serde(default)]
    pub ax_tree_json_path: Option<String>,
    #[serde(default)]
    pub exclusion_match: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample() -> SnapshotEnvelope {
        SnapshotEnvelope {
            v: ENVELOPE_VERSION,
            captured_at: Utc.with_ymd_and_hms(2026, 4, 27, 10, 14, 23).unwrap(),
            device_id: "dev-1".into(),
            session_id: "sess-1".into(),
            frame: FrameSnapshot {
                app: AppInfo {
                    bundle_id: Some("com.foo.bar".into()),
                    name: Some("Foo".into()),
                    pid: Some(123),
                },
                window: WindowInfo {
                    title: Some("hello".into()),
                    is_focused: true,
                },
                url: None,
                screenshot: Some(ScreenshotRef {
                    path: "captures/2026/04/27/abc.jpg".into(),
                    hash: "sha256:abc".into(),
                    size_bytes: Some(1234),
                }),
            },
            extraction: ExtractionResult {
                strategy: ExtractionStrategy::Ocr,
                ax_text: None,
                ocr_text: Some("text".into()),
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
    fn round_trip_minimal_envelope() {
        let env = sample();
        let json = serde_json::to_string(&env).unwrap();
        let back: SnapshotEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn ax_ocr_strategy_serializes_with_plus() {
        let s = serde_json::to_string(&ExtractionStrategy::AxOcr).unwrap();
        assert_eq!(s, "\"ax+ocr\"");
    }

    #[test]
    fn skipped_strategy_round_trips() {
        let s = serde_json::to_string(&ExtractionStrategy::Skipped).unwrap();
        assert_eq!(s, "\"skipped\"");
        let back: ExtractionStrategy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ExtractionStrategy::Skipped);
    }

    #[test]
    fn adapter_strategy_round_trips() {
        let s = serde_json::to_string(&ExtractionStrategy::Adapter).unwrap();
        assert_eq!(s, "\"adapter\"");
        let back: ExtractionStrategy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ExtractionStrategy::Adapter);
    }

    #[test]
    fn trigger_round_trips_each_variant() {
        for (variant, expected) in [
            (Trigger::FocusChange, "\"focus_change\""),
            (Trigger::FocusedWindowChanged, "\"focused_window_changed\""),
            (Trigger::TitleChanged, "\"title_changed\""),
            (Trigger::QuickAsk, "\"quick_ask\""),
            (Trigger::Manual, "\"manual\""),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected);
            let back: Trigger = serde_json::from_str(&s).unwrap();
            assert_eq!(back, variant);
        }
    }
}
