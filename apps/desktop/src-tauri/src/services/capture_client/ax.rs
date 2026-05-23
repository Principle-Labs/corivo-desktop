//! Phase 2 — `ax.*` typed wrappers around the helper's Accessibility text
//! query and selection probe.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::client::CaptureClient;
use super::error::{CaptureError, Result};
use super::protocol::methods;

/// Per-spec defaults for the AX walk.
const DEFAULT_MAX_DEPTH: u32 = 64;
const DEFAULT_MAX_CHARS: u32 = 64_000;
const DEFAULT_QUERY_DEADLINE: Duration = Duration::from_millis(1500);
const DEFAULT_PROBE_DEADLINE: Duration = Duration::from_millis(250);

/// Per-app skip predicate. Helper applies these as string equality on the
/// AX node's `role` / `subrole` and substring match on `description`.
/// Complex closure-based logic stays in Rust adapters consuming the
/// resulting text.
#[derive(Debug, Clone, Default)]
pub struct AxSkipPredicate {
    pub skip_roles: Vec<String>,
    pub skip_subroles: Vec<String>,
    pub skip_descriptions_substr: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AxQueryOpts {
    pub pid: i32,
    pub max_depth: u32,
    pub max_chars: u32,
    pub deadline: Duration,
    pub skip: AxSkipPredicate,
}

impl AxQueryOpts {
    pub fn for_pid(pid: i32) -> Self {
        Self {
            pid,
            max_depth: DEFAULT_MAX_DEPTH,
            max_chars: DEFAULT_MAX_CHARS,
            deadline: DEFAULT_QUERY_DEADLINE,
            skip: AxSkipPredicate::default(),
        }
    }

    pub fn with_skip(mut self, skip: AxSkipPredicate) -> Self {
        self.skip = skip;
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AxQueryResult {
    pub text: String,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub truncated: bool,
    /// Non-error explanation when the helper couldn't get meaningful text.
    /// `None` on the normal success path. Defined values today (from the
    /// Windows helper; macOS Swift surfaces will follow):
    ///
    /// - `"no_focused_window"` — no window for `pid` was visible to the helper
    /// - `"elevated_target"`   — target pid runs at higher integrity than the
    ///   helper, so we cannot read its UIA tree (Windows-specific signal;
    ///   surfaces with a one-time toast on the Rust side)
    /// - `"empty_tree"`        — window was found but yielded no text
    ///
    /// Older helpers omit this field; deserialization treats that as `None`.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AxProbeSelectionResult {
    pub selection: Option<String>,
}

impl CaptureClient {
    /// Walk the focused window of `pid` and return role-tagged plaintext.
    /// Format matches the existing
    /// [`crate::services::extractor::ax_extractor`] output (markers
    /// `[TITLE]` / `[BUTTON]` / `[TAB]` inline).
    pub async fn ax_query(&self, opts: AxQueryOpts) -> Result<AxQueryResult> {
        if !self.capabilities().ax_query {
            return Err(CaptureError::Unsupported(
                "helper does not support ax.query".into(),
            ));
        }
        let mut payload = json!({
            "pid": opts.pid,
            "max_depth": opts.max_depth,
            "max_chars": opts.max_chars,
            "deadline_ms": opts.deadline.as_millis() as u64,
        });
        let mut skip_obj = serde_json::Map::new();
        if !opts.skip.skip_roles.is_empty() {
            skip_obj.insert("skip_roles".into(), json!(opts.skip.skip_roles));
        }
        if !opts.skip.skip_subroles.is_empty() {
            skip_obj.insert("skip_subroles".into(), json!(opts.skip.skip_subroles));
        }
        if !opts.skip.skip_descriptions_substr.is_empty() {
            skip_obj.insert(
                "skip_descriptions_substr".into(),
                json!(opts.skip.skip_descriptions_substr),
            );
        }
        if !skip_obj.is_empty() {
            payload["skip_predicate"] = serde_json::Value::Object(skip_obj);
        }
        // Add a ~25% client-side margin on top of the helper deadline so a
        // helper that races right at its budget doesn't fail on the wire.
        let client_deadline = opts.deadline + opts.deadline / 4;
        self.request(methods::AX_QUERY, Some(payload), client_deadline)
            .await
    }

    /// Probe the user's currently-highlighted text in `pid`'s focused
    /// app. Returns `None` if nothing is selected (the common case).
    pub async fn ax_probe_selection(&self, pid: i32) -> Result<Option<String>> {
        if !self.capabilities().ax_query {
            return Err(CaptureError::Unsupported(
                "helper does not support ax.probe_selection".into(),
            ));
        }
        let payload = json!({
            "pid": pid,
            "deadline_ms": DEFAULT_PROBE_DEADLINE.as_millis() as u64,
        });
        let client_deadline = DEFAULT_PROBE_DEADLINE + DEFAULT_PROBE_DEADLINE / 4;
        let result: AxProbeSelectionResult = self
            .request(methods::AX_PROBE_SELECTION, Some(payload), client_deadline)
            .await?;
        Ok(result.selection)
    }
}
