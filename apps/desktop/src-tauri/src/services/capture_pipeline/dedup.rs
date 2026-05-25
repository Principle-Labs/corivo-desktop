//! Pure-function dedup helpers — capture pipeline calls these between
//! "I have a probe" and "I'm about to write a frame".
//!
//! Metadata-only ladder: periodic screenshot capture is gone, so the
//! second-stage screenshot-hash compare from the v3 spec doesn't apply.
//! If `(app_bundle_id, window_title, url)` match the last frame in
//! this session → bump `still_present_until`, skip extract + insert.
//! Anything else → write fresh.

use crate::domain::frame::Frame;

/// Metadata snapshot taken from the foreground probe.
///
/// All three fields are borrowed `Option<&str>` so callers can pass slices
/// from a `ForegroundProbe` without cloning.
#[derive(Debug, Clone, Copy)]
pub struct CurrentMetadata<'a> {
    pub app_bundle_id: Option<&'a str>,
    pub window_title: Option<&'a str>,
    pub url: Option<&'a str>,
}

/// Decision the dedup ladder produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupDecision {
    /// No previous frame in this session, or metadata changed → run the
    /// full extract + write pipeline.
    WriteFresh,
    /// Metadata matches the last frame → bump still_present_until on
    /// the last frame; skip extract + insert.
    BumpStillPresent { last_frame_id: String },
}

/// Compare metadata to the last frame in this capture session.
pub fn classify_metadata(last: Option<&Frame>, current: &CurrentMetadata<'_>) -> DedupDecision {
    let Some(last) = last else {
        return DedupDecision::WriteFresh;
    };
    if metadata_matches(last, current) {
        DedupDecision::BumpStillPresent {
            last_frame_id: last.id.clone(),
        }
    } else {
        DedupDecision::WriteFresh
    }
}

fn metadata_matches(last: &Frame, current: &CurrentMetadata<'_>) -> bool {
    last.app_bundle_id.as_deref() == current.app_bundle_id
        && last.window_title.as_deref() == current.window_title
        && last.url.as_deref() == current.url
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_frame(
        id: &str,
        bundle: Option<&str>,
        window: Option<&str>,
        url: Option<&str>,
    ) -> Frame {
        Frame {
            id: id.to_string(),
            captured_at: Utc.with_ymd_and_hms(2026, 4, 27, 10, 0, 0).unwrap(),
            device_id: "dev".into(),
            capture_session_id: "sess".into(),
            app_bundle_id: bundle.map(String::from),
            app_name: None,
            window_title: window.map(String::from),
            url: url.map(String::from),
            screenshot_path: None,
            screenshot_hash: None,
            screenshot_size_bytes: None,
            ax_text: None,
            ocr_text: None,
            adapter_name: None,
            adapter_payload: None,
            extraction_strategy: "ax".into(),
            extraction_duration_ms: None,
            fallback_reason: None,
            trigger: "focus_change".into(),
            content_hash: None,
            derived_from_frame_id: None,
            still_present_until: None,
            exclusion_match: None,
            search_tokens: None,
            created_at: Utc.with_ymd_and_hms(2026, 4, 27, 10, 0, 0).unwrap(),
        }
    }

    #[test]
    fn no_last_frame_writes_fresh() {
        let current = CurrentMetadata {
            app_bundle_id: Some("a"),
            window_title: None,
            url: None,
        };
        assert_eq!(classify_metadata(None, &current), DedupDecision::WriteFresh);
    }

    #[test]
    fn metadata_change_writes_fresh() {
        let last = make_frame("L", Some("a"), Some("w"), None);
        let current = CurrentMetadata {
            app_bundle_id: Some("b"),
            window_title: Some("w"),
            url: None,
        };
        assert_eq!(
            classify_metadata(Some(&last), &current),
            DedupDecision::WriteFresh
        );
    }

    #[test]
    fn metadata_match_bumps_still_present() {
        let last = make_frame("L", Some("a"), None, None);
        let current = CurrentMetadata {
            app_bundle_id: Some("a"),
            window_title: None,
            url: None,
        };
        assert_eq!(
            classify_metadata(Some(&last), &current),
            DedupDecision::BumpStillPresent {
                last_frame_id: "L".into(),
            },
        );
    }
}
