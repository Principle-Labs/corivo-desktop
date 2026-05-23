//! App-update policy source.
//!
//! The Tauri updater plugin itself is build-time configured by each
//! distribution. This trait abstracts the *policy* around that —
//! minimum-version gating, channel selection, the "are we managed?"
//! flag the UI uses to decide whether to show a "检查更新" button or a
//! passive "前往 GitHub 下载" link.
//!
//! ### Implementations
//!
//! * **`cloud::corivo::updater_policy`** — fetches
//!   a closed-backend update policy endpoint and
//!   enforces the `minVersion` block. Bundled with the
//!   `corivo-cloud` feature.
//!
//! * **`noop::NoopUpdaterPolicyService`** — `is_managed() == false`,
//!   `fetch_policy()` returns `Ok(None)`. The OSS build still uses
//!   the Tauri updater plugin but points it at GitHub Releases via
//!   the build-time `tauri.conf.json` (no policy gate).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Result;

/// Subset of the server policy the desktop needs. Mirrors the fields
/// the frontend `updater.ts::UpdatePolicy` already consumes today;
/// once Task #8 wires the typed command this becomes the canonical
/// home for the shape.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct UpdatePolicy {
    pub channel: String,
    pub min_version: String,
    pub latest_version: String,
    pub updated_at: String,
}

#[async_trait]
pub trait UpdaterPolicyService: Send + Sync {
    /// True when there's a managed policy endpoint to call. OSS build
    /// = false (the user just gets whatever the Tauri updater plugin's
    /// GitHub-Releases endpoint says is latest, no min-version gate).
    fn is_managed(&self) -> bool {
        false
    }

    /// Fetch the current policy. `Ok(None)` means "no policy on file"
    /// — for the OSS noop this is the always-answer; for the corivo
    /// impl it indicates the policy endpoint succeeded but had no
    /// active record for this channel (e.g. brand-new env), in which
    /// case the UI falls back to "always allow update".
    async fn fetch_policy(&self) -> Result<Option<UpdatePolicy>>;
}
