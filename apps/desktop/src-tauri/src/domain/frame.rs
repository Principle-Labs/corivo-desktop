//! `frames` table row + insert builder (spec §五).
//!
//! The DB row mirrors `SnapshotEnvelope` for storage: where the envelope
//! groups fields nestedly (app / window / extraction), the row flattens
//! them so SQL queries don't need JSON path operators.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    pub id: String,
    pub captured_at: DateTime<Utc>,
    pub device_id: String,
    pub capture_session_id: String,

    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,

    pub screenshot_path: Option<String>,
    pub screenshot_hash: Option<String>,
    pub screenshot_size_bytes: Option<i64>,

    pub ax_text: Option<String>,
    pub ocr_text: Option<String>,
    /// v1500 (docs/privacy-filter-spec.md §5): PII spans 元数据。
    /// `Some(json)` = 已经 classify 完，JSON 数组形态见
    /// [`crate::domain::privacy::PiiSpan`]。空数组 `"[]"` 与 `None`
    /// 语义不同 —— `None` 是"还没跑过"，空数组是"跑过了无 PII"。
    /// 当数组里有 `label="secret"` 且 `redacted_in_storage=true` 的
    /// span 时，`ax_text` 对应区间已被物理替换。
    pub ax_text_pii_spans: Option<String>,
    /// Phase 5: per-app adapter that produced this row, e.g. `chrome`,
    /// `vscode`, `lark`, `generic_ax`. `None` for Phase 1-4 rows
    /// (legacy AX / OCR / skipped paths).
    pub adapter_name: Option<String>,
    /// Phase 5: JSON payload the adapter emits — URL / file path /
    /// selection / cwd / channel etc. Stored as a String so the repo
    /// can stay agnostic of which adapter produced it.
    pub adapter_payload: Option<String>,
    pub extraction_strategy: String,
    pub extraction_duration_ms: Option<i64>,
    pub fallback_reason: Option<String>,
    /// What woke capture_pipeline up: 'timer' / 'focus_change' / 'quick_ask'.
    pub trigger: String,

    pub content_hash: Option<String>,
    pub derived_from_frame_id: Option<String>,
    pub still_present_until: Option<DateTime<Utc>>,

    pub exclusion_match: Option<String>,

    /// v600: jieba-tokenized index column. Read for diagnostics; the
    /// recall layer never queries this directly — it goes through
    /// `frames_fts MATCH …`.
    pub search_tokens: Option<String>,

    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewFrame {
    pub captured_at: DateTime<Utc>,
    pub device_id: String,
    pub capture_session_id: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub screenshot_path: Option<String>,
    pub screenshot_hash: Option<String>,
    pub screenshot_size_bytes: Option<i64>,
    pub ax_text: Option<String>,
    pub ocr_text: Option<String>,
    /// v1500: privacy-filter spans. Capture pipeline 默认填 `None`；
    /// snapshot_consumer 在 insert 之前调 PrivacyFilter::classify_*
    /// 拿到 spans 后再回写。详见 docs/privacy-filter-spec.md §7.1。
    pub ax_text_pii_spans: Option<String>,
    pub adapter_name: Option<String>,
    pub adapter_payload: Option<String>,
    pub extraction_strategy: String,
    pub extraction_duration_ms: Option<i64>,
    pub fallback_reason: Option<String>,
    pub trigger: String,
    pub content_hash: Option<String>,
    pub exclusion_match: Option<String>,
    /// v600: pre-tokenized text for FTS5. Computed at envelope ingest
    /// via `services::tokenize::tokenize_for_index`.
    pub search_tokens: Option<String>,
}

impl NewFrame {
    /// Defensive default for `trigger`. The capture pipeline always
    /// stamps an explicit trigger from the event coordinator —
    /// `manual` is just a "shouldn't happen on real envelopes"
    /// fallback so the CHECK constraint never trips on a partially
    /// constructed test fixture.
    pub fn default_trigger() -> &'static str {
        "manual"
    }
}
