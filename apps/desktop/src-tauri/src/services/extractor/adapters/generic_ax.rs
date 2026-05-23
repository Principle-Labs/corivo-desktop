//! `GenericAxAdapter` — fallback adapter used when no per-app adapter
//! claims the bundle id. Calls the cross-app `ax_extractor::extract_full`
//! (a recursive AX walk with role-tagged plaintext + the helper's
//! `reason` signal) and decides whether the result is good enough to use
//! or whether the dispatcher should fall through to OCR.
//!
//! Decision ladder (in order):
//!
//! 1. **Helper-signalled fallback.** If `AxExtraction.reason` is
//!    `Some(..)` — e.g. `"elevated_target"` on Windows when the target
//!    runs at higher integrity, `"no_focused_window"` when no window
//!    matched the pid, `"empty_tree"` when the window has no readable
//!    UIA nodes — the adapter returns `NeedsOcr` and forwards that
//!    reason verbatim (so the frame row records why AX didn't work).
//! 2. **AX too short.** Text shorter than `AX_MIN_CHARS` chars (spec
//!    §六 condition ②) is treated as "AX failed" and returns
//!    `NeedsOcr` with reason `"ax_too_short"`.
//! 3. **AX errored.** Helper unavailable / IPC error / OS rejection
//!    returns `NeedsOcr` with reason `"ax_failed"`.
//! 4. **AX OK.** Otherwise returns `Extracted`.
//!
//! This adapter is the only place where `ax_extractor::extract_full`
//! is invoked — the dispatcher is pure routing on `AdapterOutcome`.

use async_trait::async_trait;
use serde_json::json;

use super::super::error::Result;
use super::{AdapterContext, AdapterOutcome, FocusAdapter};

/// Stable name written into `frames.adapter_name` when this adapter
/// runs. Exposed so the dispatcher can detect "this is the generic
/// path" without comparing string literals.
pub const NAME: &str = "generic_ax";

/// Minimum AX text length for AX to be considered "successful". Below
/// this we fall back to OCR (spec §六 fallback condition ②).
pub(crate) const AX_MIN_CHARS: usize = 30;

pub struct GenericAxAdapter;

impl GenericAxAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenericAxAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FocusAdapter for GenericAxAdapter {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, _bundle_id: &str) -> bool {
        true
    }

    async fn extract(&self, ctx: &AdapterContext<'_>) -> Result<AdapterOutcome> {
        use super::super::ax_extractor::{self, SkipPredicate};

        let ax_result = ax_extractor::extract_full(ctx.pid, SkipPredicate::default()).await;

        match ax_result {
            Ok(extraction) => {
                // 1. Helper-signalled fallback wins over text-length heuristics.
                if let Some(reason) = extraction.reason {
                    tracing::debug!(
                        ax_chars = extraction.text.chars().count(),
                        helper_reason = %reason,
                        "generic_ax.helper_signalled_fallback"
                    );
                    return Ok(AdapterOutcome::NeedsOcr {
                        payload: json!({ "kind": NAME }),
                        reason,
                    });
                }
                // 2. Helper succeeded — apply the AX_MIN_CHARS heuristic.
                let chars = extraction.text.chars().count();
                if chars >= AX_MIN_CHARS {
                    tracing::debug!(ax_chars = chars, "generic_ax.extracted");
                    Ok(AdapterOutcome::Extracted {
                        text: extraction.text,
                        payload: json!({ "kind": NAME }),
                    })
                } else {
                    tracing::debug!(
                        ax_chars = chars,
                        threshold = AX_MIN_CHARS,
                        "generic_ax.ax_too_short"
                    );
                    Ok(AdapterOutcome::NeedsOcr {
                        payload: json!({ "kind": NAME }),
                        reason: "ax_too_short".into(),
                    })
                }
            }
            Err(error) => {
                // 3. Helper unavailable / IPC error / OS rejection.
                tracing::debug!(?error, "generic_ax.ax_failed");
                Ok(AdapterOutcome::NeedsOcr {
                    payload: json!({ "kind": NAME }),
                    reason: "ax_failed".into(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_generic_ax() {
        assert_eq!(GenericAxAdapter::new().name(), "generic_ax");
        assert_eq!(NAME, "generic_ax");
    }

    #[test]
    fn matches_anything() {
        let a = GenericAxAdapter::new();
        assert!(a.matches(""));
        assert!(a.matches("com.example.MyApp"));
    }

    #[test]
    fn ax_min_chars_matches_spec() {
        // Spec §六 condition ② locks this number; bumping it should
        // require an explicit decision recorded in the spec.
        assert_eq!(AX_MIN_CHARS, 30);
    }
}
