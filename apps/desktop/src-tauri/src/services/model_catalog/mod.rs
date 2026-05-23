//! Model directory cache.
//!
//! Mirrors the corivo backend's `GET /v1/me/models` response (see
//! `apps/api/src/routes/me.ts`) into `$APPDATA/.models-cache.json` so
//! the Settings picker can open instantly and so the chat path has a
//! stable reference even when the network is offline.
//!
//! The response is per-user. Each alias the admin has granted carries
//! its own (api_host, api_key) issued by sub2api at grant time —
//! different aliases may belong to different sub2api groups, so the
//! credentials are per-entry rather than shared at the directory
//! level. Each entry also carries the `upstream_model` id the sidecar
//! sends to sub2api and the `client_protocol` the agent uses to pick
//! its adapter (today: openai or anthropic — sub2api normalizes
//! azure / gemini into the openai shape upstream).
//!
//! Lifecycle:
//!   * remote refresh / alias sync live behind the cloud session trait
//!     so the Corivo-specific HTTP contract stays in the private overlay.
//!   * `load_cached()` — sync disk read. Returns the seeded demo set
//!     when the cache is missing so `pnpm app:dev` still works without
//!     the backend up.
//!
//! The catalog file is **not** part of `Config` — its contents change
//! without user intent and we don't want them to roundtrip through
//! `set_config`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

use crate::domain::config::ApiShape;
use crate::error::{CorivoError, Result};

const CACHE_FILENAME: &str = ".models-cache.json";

/// Upstream provider classification. Informational at the API surface
/// — the agent's adapter choice uses [`ModelMeta::client_protocol`].
/// Stored as a string rather than a fixed enum so the admin can add
/// new upstreams (e.g. "bedrock") without requiring a desktop bump.
pub type ApiType = String;

/// Capability flags the sidecar + picker UI read at runtime.
/// Stored as JSON on the backend so adding fields doesn't require a
/// migration. Every field carries `#[serde(default)]` so an empty
/// `{}` (admin seeded only the model row, didn't fill capabilities
/// yet) still parses — UI just shows the alias with no badges and
/// `0k context`, which is the right "tell me to fix the data"
/// signal rather than a wholesale fetch failure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case", default)]
pub struct ModelCapabilities {
    pub reasoning: bool,
    pub tool_use: bool,
    pub vision: bool,
    pub context_window: u32,
    pub max_tokens: u32,
}

/// One alias in the admin catalog. Single-key architecture: every
/// account sees every enabled alias; credentials live on the
/// directory (top-level `host` / `api_key`) and the sub2api key's
/// group is what determines which upstream model gets hit. The
/// picker calls `models_set_active_model` to flip groups.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub struct ModelMeta {
    /// Stable client-facing identifier (e.g. `"corivo:fast"`).
    pub alias: String,
    pub display_name: String,
    /// True upstream provider — informational, may be used for badges.
    pub api_type: ApiType,
    /// Protocol the agent client speaks for this alias. sub2api
    /// normalizes azure / gemini into openai, so today this is always
    /// `Openai` or `Anthropic`.
    pub client_protocol: ApiShape,
    /// Upstream model id sub2api routes on. Sent in the chat
    /// completion `model` field.
    pub upstream_model: String,
    pub capabilities: ModelCapabilities,
}

/// `/v1/me/models` response. `host` + `api_key` are the per-user
/// sub2api credentials (single key — see `accounts.api_host` /
/// `apiKey` on the backend); `active_alias` is the alias the user
/// has currently switched to. `managed[]` is the global catalog
/// available for switching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub struct ModelDirectory {
    pub host: String,
    pub api_key: String,
    pub default_alias: String,
    pub active_alias: String,
    pub managed: Vec<ModelMeta>,
    /// Backend's `version` field — RFC3339 of the latest catalog
    /// `updated_at`. Used by the UI for a "last refreshed" hint.
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheFile {
    /// Unix epoch milliseconds. Lets the UI render a "last refreshed"
    /// hint even when `version` is missing from an older cache.
    fetched_at: i64,
    directory: ModelDirectory,
}

/// Seeded demo so `pnpm app:dev` shows *something* without the
/// backend up. The fake `api_key` is intentionally a clearly-broken
/// placeholder — if it ever reaches a real upstream the 401 will
/// land immediately, which is exactly the right signal that the user
/// is not logged in yet.
fn seeded_demo_directory() -> ModelDirectory {
    ModelDirectory {
        host: "https://sub2api.invalid".to_string(),
        api_key: "demo-please-login".to_string(),
        default_alias: "corivo:demo".to_string(),
        active_alias: "corivo:demo".to_string(),
        managed: vec![ModelMeta {
            alias: "corivo:demo".to_string(),
            display_name: "Demo · 请登录".to_string(),
            api_type: "openai".to_string(),
            client_protocol: ApiShape::Openai,
            upstream_model: "gpt-4o-mini".to_string(),
            capabilities: ModelCapabilities {
                reasoning: false,
                tool_use: true,
                vision: true,
                context_window: 128_000,
                max_tokens: 16_384,
            },
        }],
        version: "0".to_string(),
    }
}

/// Stateless cache wrapper — owns the on-disk path.
pub struct ModelCatalog {
    cache_path: PathBuf,
}

impl ModelCatalog {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let cache_path = app_data_dir.join(CACHE_FILENAME);
        Self { cache_path }
    }

    /// Disk read. Returns the seeded demo directory when the cache
    /// file is missing / empty / unparseable.
    pub fn load_cached(&self) -> ModelDirectory {
        match read_cache_file(&self.cache_path) {
            Ok(Some(file)) if !file.directory.managed.is_empty() => file.directory,
            Ok(_) => seeded_demo_directory(),
            Err(error) => {
                tracing::warn!(?error, "model_catalog.read_cache_failed");
                seeded_demo_directory()
            }
        }
    }

    /// Convenience for callers that only want the alias list (Settings
    /// picker, completion lookups). Equivalent to
    /// `load_cached().managed`.
    pub fn load_cached_aliases(&self) -> Vec<ModelMeta> {
        self.load_cached().managed
    }

    /// Look up a single alias from the cached directory. Returns
    /// `None` if the alias has been revoked / disabled since the last
    /// refresh — callers should fall back to `default_alias` in that
    /// case.
    pub fn find_alias(&self, alias: &str) -> Option<ModelMeta> {
        self.load_cached()
            .managed
            .into_iter()
            .find(|m| m.alias == alias)
    }

    /// Write a new `active_alias` into the cached directory file.
    /// Called by `models_set_active_model` after the backend's
    /// `POST /v1/me/active-model` returns 200 — keeps the on-disk
    /// cache in sync with the gateway without waiting for the next
    /// refresh. No-op if there's no cache yet (the next refresh will
    /// pick up the right active_alias from the backend anyway).
    pub fn set_active_alias_in_cache(&self, alias: &str) -> Result<()> {
        let mut directory = self.load_cached();
        directory.active_alias = alias.to_string();
        self.save_cache(&directory)
    }

    /// Resolve the alias the user has currently switched to. Returns
    /// the cached directory's `active_alias` entry, or falls back to
    /// `default_alias`, or to the first managed entry. Returns `None`
    /// only when the cache is empty (pre-login).
    pub fn active_entry(&self) -> Option<ModelMeta> {
        let dir = self.load_cached();
        dir.managed
            .iter()
            .find(|m| m.alias == dir.active_alias)
            .or_else(|| dir.managed.iter().find(|m| m.alias == dir.default_alias))
            .or_else(|| dir.managed.first())
            .cloned()
    }

    /// Public so the boot path / login hook can seed the cache from a
    /// synchronous source if the backend isn't reachable.
    pub fn save_cache(&self, directory: &ModelDirectory) -> Result<()> {
        let payload = CacheFile {
            fetched_at: now_ms(),
            directory: directory.clone(),
        };
        if let Some(parent) = self.cache_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CorivoError::Internal(format!(
                    "creating model cache parent {} failed: {e}",
                    parent.display()
                ))
            })?;
        }
        let buf = serde_json::to_vec_pretty(&payload)
            .map_err(|e| CorivoError::Internal(format!("model cache serialize failed: {e}")))?;
        std::fs::write(&self.cache_path, buf).map_err(|e| {
            CorivoError::Internal(format!(
                "writing model cache to {} failed: {e}",
                self.cache_path.display()
            ))
        })?;
        Ok(())
    }
}

fn read_cache_file(path: &Path) -> Result<Option<CacheFile>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let file: CacheFile = serde_json::from_slice(&bytes)
                .map_err(|e| CorivoError::Internal(format!("model cache parse failed: {e}")))?;
            Ok(Some(file))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CorivoError::Internal(format!(
            "reading model cache {} failed: {e}",
            path.display()
        ))),
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_cached_falls_back_to_seeded_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let cat = ModelCatalog::new(dir.path().to_path_buf());
        let directory = cat.load_cached();
        assert!(!directory.managed.is_empty());
        assert_eq!(directory.default_alias, "corivo:demo");
    }

    fn fixture_entry(alias: &str, upstream: &str) -> ModelMeta {
        ModelMeta {
            alias: alias.to_string(),
            display_name: alias.to_string(),
            api_type: "openai".to_string(),
            client_protocol: ApiShape::Openai,
            upstream_model: upstream.to_string(),
            capabilities: ModelCapabilities {
                reasoning: false,
                tool_use: true,
                vision: true,
                context_window: 128_000,
                max_tokens: 16_384,
            },
        }
    }

    fn fixture_directory(active: &str, aliases: &[&str]) -> ModelDirectory {
        ModelDirectory {
            host: "https://sub2api.example.com".to_string(),
            api_key: "sk-test".to_string(),
            default_alias: aliases.first().copied().unwrap_or(active).to_string(),
            active_alias: active.to_string(),
            managed: aliases
                .iter()
                .map(|a| fixture_entry(a, &format!("{a}-upstream")))
                .collect(),
            version: "0".to_string(),
        }
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cat = ModelCatalog::new(dir.path().to_path_buf());
        let custom = fixture_directory("corivo:fast", &["corivo:fast", "corivo:slow"]);
        cat.save_cache(&custom).unwrap();
        assert_eq!(cat.load_cached(), custom);
    }

    #[test]
    fn find_alias_returns_match_or_none() {
        let dir = TempDir::new().unwrap();
        let cat = ModelCatalog::new(dir.path().to_path_buf());
        cat.save_cache(&fixture_directory("a", &["a"])).unwrap();
        assert_eq!(
            cat.find_alias("a").map(|m| m.upstream_model),
            Some("a-upstream".to_string()),
        );
        assert!(cat.find_alias("missing").is_none());
    }

    #[test]
    fn active_entry_prefers_active_then_default_then_first() {
        let dir = TempDir::new().unwrap();
        let cat = ModelCatalog::new(dir.path().to_path_buf());

        // active = "b" (exists)
        cat.save_cache(&fixture_directory("b", &["a", "b", "c"]))
            .unwrap();
        assert_eq!(cat.active_entry().map(|m| m.alias), Some("b".into()));

        // active = "zzz" (missing) → fall back to default (= "a", the first)
        cat.save_cache(&fixture_directory("zzz", &["a", "b", "c"]))
            .unwrap();
        assert_eq!(cat.active_entry().map(|m| m.alias), Some("a".into()));
    }
}
