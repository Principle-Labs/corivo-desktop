//! AX (Accessibility API) text extraction.
//!
//! **Phase 6 migration**: the synchronous AX walk used to live inline
//! using `accessibility-sys`. The walk now lives in the capture helper
//! sidecar (Swift on macOS, UIA on Windows). The public API is preserved
//! so per-app adapters don't need to change shape — only `extract_with_skip`'s
//! predicate goes from a closure to a static [`SkipPredicate`] (the
//! helper applies the predicate over IPC and can't run arbitrary Rust
//! closures).
//!
//! `is_process_trusted` stays as a direct `AXIsProcessTrusted` call —
//! it's a single C API with no state, used by the permissions UI to
//! show "Accessibility: granted/missing" without round-tripping through
//! the helper. accessibility-sys remains a dep for that one symbol (and
//! for the still-Rust `ax_observer`).

use crate::services::capture_client::{self, AxQueryOpts, AxSkipPredicate};

use super::error::{ExtractorError, Result};

/// Static skip predicate used by per-app adapters. Matches the shape of
/// [`crate::services::capture_client::AxSkipPredicate`] one-to-one.
#[derive(Debug, Clone, Default)]
pub struct SkipPredicate {
    pub skip_roles: Vec<String>,
    pub skip_subroles: Vec<String>,
    pub skip_descriptions_substr: Vec<String>,
}

impl From<SkipPredicate> for AxSkipPredicate {
    fn from(value: SkipPredicate) -> Self {
        AxSkipPredicate {
            skip_roles: value.skip_roles,
            skip_subroles: value.skip_subroles,
            skip_descriptions_substr: value.skip_descriptions_substr,
        }
    }
}

/// Full helper response: role-tagged plaintext plus the helper's
/// `reason` field. `reason` is `Some(..)` when the helper couldn't get
/// meaningful text — see [`crate::services::capture_client::AxQueryResult::reason`]
/// for the list of values. Callers that want to make a fallback decision
/// (e.g. trigger OCR) should branch on `reason` first, then on text length.
#[derive(Debug, Clone)]
pub struct AxExtraction {
    pub text: String,
    pub reason: Option<String>,
}

/// Extract role-tagged plaintext from the frontmost window of `pid`.
///
/// Returns the empty string if the AX walk found no extractable text;
/// returns `Err(ExtractorError::Failed)` if the helper is unavailable
/// or the OS rejected the call (no permission, invalid pid, …).
///
/// Callers that need the helper's `reason` field (to drive a fallback
/// decision) should use [`extract_full`] instead.
pub async fn extract(pid: Option<i32>) -> Result<String> {
    extract_with_skip(pid, SkipPredicate::default()).await
}

/// Same as [`extract`] but with a per-call skip predicate. Used by
/// per-app adapters (Lark / etc.) to prune navigation chrome.
pub async fn extract_with_skip(pid: Option<i32>, skip: SkipPredicate) -> Result<String> {
    extract_full(pid, skip).await.map(|e| e.text)
}

/// Full version of [`extract_with_skip`] that surfaces the helper's
/// `reason` field alongside the text. Used by the dispatcher's generic
/// adapter to decide whether to fall back to OCR.
pub async fn extract_full(pid: Option<i32>, skip: SkipPredicate) -> Result<AxExtraction> {
    let Some(pid) = pid else {
        return Err(ExtractorError::Failed(
            "ax_extractor: no pid for foreground app".into(),
        ));
    };

    let client = capture_client::global::get()
        .map_err(|e| ExtractorError::Internal(format!("ax_extractor: helper unavailable: {e}")))?;

    let opts = AxQueryOpts::for_pid(pid).with_skip(skip.into());
    let result = client
        .ax_query(opts)
        .await
        .map_err(|e| ExtractorError::Failed(format!("ax_extractor: helper rejected: {e}")))?;

    Ok(AxExtraction {
        text: result.text,
        reason: result.reason,
    })
}

/// Whether the OS has granted Accessibility permission to the helper
/// bundle (macOS) — UIA on Windows needs no per-process permission so
/// this always returns `true` there. Phase 7 routes through the helper's
/// `permission.status` RPC so we can drop the last accessibility-sys
/// usage on the Rust side; if the helper is unavailable we fail-open
/// with `true` so the UI doesn't show a misleading "permission missing"
/// banner during a degraded boot.
pub async fn is_process_trusted() -> bool {
    let Some(client) = capture_client::global::try_get() else {
        return true;
    };
    match client.permission_status().await {
        Ok(status) => status.accessibility.is_granted(),
        Err(error) => {
            tracing::trace!(
                ?error,
                "is_process_trusted: helper rejected, treating as granted"
            );
            true
        }
    }
}
