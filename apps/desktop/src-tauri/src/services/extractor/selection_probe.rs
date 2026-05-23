//! AX-based selection probe.
//!
//! **Phase 6 migration**: the AXTextMarker walk used to live inline using
//! `accessibility-sys`. It now lives in the capture helper sidecar
//! (`SelectionProbe.swift` on macOS / UIA TextPattern on Windows). Public
//! surface unchanged: returns `None` for any failure (missing perm /
//! no selection / ancestor walk exhausted / IPC error) so callers can
//! ignore selection silently when it's not available.

use crate::services::capture_client;

/// Convenience wrapper for the common `Option<i32>` shape callers
/// already carry (probe foreground pid → Option<selection>).
pub async fn probe(pid: Option<i32>) -> Option<String> {
    let pid = pid?;
    probe_for_pid(pid).await
}

/// Probe the user's currently-highlighted text in `pid`'s focused
/// app. Returns `None` for every failure mode — the caller surfaces
/// "no selection" identically whether the OS API said so or the helper
/// is unavailable.
pub async fn probe_for_pid(pid: i32) -> Option<String> {
    let client = capture_client::global::try_get()?;
    match client.ax_probe_selection(pid).await {
        Ok(sel) => sel,
        Err(error) => {
            tracing::trace!(
                ?error,
                pid,
                "selection_probe: helper rejected (treating as no-selection)"
            );
            None
        }
    }
}
