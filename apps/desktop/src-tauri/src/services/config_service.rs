use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use serde_json::Value;
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt};

use crate::{
    domain::config::{Config, ConnectorsConfig, Language},
    error::{CorivoError, Result},
};

const STORE_FILENAME: &str = "config.json";
const CONFIG_KEY: &str = "config";

pub trait ConfigStoreBackend: Send + Sync {
    fn get_json(&self, key: &str) -> Result<Option<Value>>;
    fn set_json(&self, key: &str, value: Value) -> Result<()>;
}

pub struct TauriConfigStore {
    store: Arc<Store<Wry>>,
}

impl TauriConfigStore {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let store = app
            .store(STORE_FILENAME)
            .map_err(|error| CorivoError::Config(error.to_string()))?;

        Ok(Self { store })
    }
}

impl ConfigStoreBackend for TauriConfigStore {
    fn get_json(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.store.get(key))
    }

    fn set_json(&self, key: &str, value: Value) -> Result<()> {
        self.store.set(key, value);
        self.store
            .save()
            .map_err(|error| CorivoError::Config(error.to_string()))?;
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryConfigStore {
    values: RwLock<HashMap<String, Value>>,
}

impl InMemoryConfigStore {
    pub fn snapshot_value(&self, key: &str) -> Option<Value> {
        self.values.read().ok()?.get(key).cloned()
    }
}

impl ConfigStoreBackend for InMemoryConfigStore {
    fn get_json(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.values.read().unwrap().get(key).cloned())
    }

    fn set_json(&self, key: &str, value: Value) -> Result<()> {
        self.values.write().unwrap().insert(key.to_string(), value);
        Ok(())
    }
}

pub struct ConfigService {
    backend: Arc<dyn ConfigStoreBackend>,
    cache: RwLock<Config>,
}

impl ConfigService {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let backend = Arc::new(TauriConfigStore::new(app)?);
        Self::with_backend(backend)
    }

    pub fn with_backend(backend: Arc<dyn ConfigStoreBackend>) -> Result<Self> {
        let config = Self::load_from_store(backend.as_ref())?;
        Ok(Self {
            backend,
            cache: RwLock::new(config),
        })
    }

    fn load_from_store(backend: &dyn ConfigStoreBackend) -> Result<Config> {
        match backend.get_json(CONFIG_KEY)? {
            Some(value) => {
                // Two-stage migration:
                //   1. serde aliases on renamed/removed fields handle the
                //      common case (see `domain::config::ExecAgentAuthMode`
                //      for the convention). They preserve user data.
                //   2. If parse still fails — e.g. a structural rewrite
                //      no alias can cover — fall back to defaults loudly
                //      rather than panicking. The user loses settings but
                //      the app boots; we surface a tracing::error so this
                //      never goes unnoticed in dev / Sentry.
                let mut config: Config = match serde_json::from_value::<Config>(value) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "config.json parse failed beyond serde-alias migration; \
                             resetting to Config::default(). User settings lost — \
                             add a serde alias or schema migration before next release."
                        );
                        Config::default()
                    }
                };
                migrate_legacy_connectors(&mut config);
                let normalized = config.normalize();
                let validated = if let Err(error) = normalized.validate() {
                    tracing::warn!(
                        %error,
                        "config validate failed; falling back to Config::default()"
                    );
                    Config::default().normalize()
                } else {
                    normalized
                };
                backend.set_json(CONFIG_KEY, serde_json::to_value(&validated)?)?;
                Ok(validated)
            }
            None => {
                // First launch — no config.json on disk yet. Greet the
                // user in their OS language instead of the static
                // English default. Both `ui_language` (UI dictionary)
                // and `response_language` (LLM reply language) are
                // seeded from the same locale; the user can override
                // either independently from Settings → General once
                // they're inside the app.
                let mut default = Config::default();
                let detected = Language::detect_from_os();
                default.app.ui_language = detected;
                default.app.response_language = detected;
                let default = default.normalize();
                backend.set_json(CONFIG_KEY, serde_json::to_value(&default)?)?;
                Ok(default)
            }
        }
    }

    pub fn get(&self) -> Config {
        self.cache.read().unwrap().clone()
    }

    pub fn update(&self, new_config: Config) -> Result<()> {
        let new_config = new_config.normalize();
        self.backend
            .set_json(CONFIG_KEY, serde_json::to_value(&new_config)?)?;
        *self.cache.write().unwrap() = new_config;
        Ok(())
    }
}

/// One-shot migration for the connector schema flip in commit 3ffab0a.
/// Old configs deserialize cleanly into the new struct — `serde(default)`
/// fills in an empty `bindings` map and `HashMap<String, _>` accepts the
/// legacy connector-id keys without complaint — but the resulting state
/// is unusable: `resolve_bound_account()` reads `bindings` first and
/// always returns `None`, so every connector card shows "未连接". Detect
/// by any account key missing the `<provider>:<account_id>` colon and
/// reset the whole connectors block. Keychain naming changed in the same
/// commit, so re-OAuth is required either way.
fn migrate_legacy_connectors(config: &mut Config) {
    let has_legacy_keys = config.connectors.accounts.keys().any(|k| !k.contains(':'));
    if has_legacy_keys {
        tracing::warn!(
            account_keys = ?config.connectors.accounts.keys().collect::<Vec<_>>(),
            "legacy connectors schema detected; resetting. Re-connect any integrations from Settings."
        );
        config.connectors = ConnectorsConfig::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::ExecAgentAuthMode;

    /// Build a minimal config.json shape carrying a legacy auth_mode value.
    /// We feed it through ConfigService::with_backend (the same path the
    /// real app uses at boot) and assert the loaded Config sees the new
    /// canonical variant. Mirrors the pre-Phase-C config schema we
    /// observed in the wild on May 8 2026 (corivo_login + obsolete
    /// anthropic_api_key field).
    fn legacy_config_json(auth_mode: &str) -> serde_json::Value {
        serde_json::json!({
            "exec_agent": {
                "auth_mode": auth_mode,
                // Pre-Phase-C field; serde silently drops unknown fields,
                // so this should NOT cause parse failure.
                "anthropic_api_key": null,
            }
        })
    }

    fn load_with_legacy(auth_mode: &str) -> Config {
        let backend = Arc::new(InMemoryConfigStore::default());
        backend
            .set_json(CONFIG_KEY, legacy_config_json(auth_mode))
            .unwrap();
        let svc = ConfigService::with_backend(backend).expect("config load must not panic");
        svc.get()
    }

    #[test]
    fn legacy_corivo_login_aliases_to_corivo_proxy() {
        let cfg = load_with_legacy("corivo_login");
        assert_eq!(cfg.exec_agent.auth_mode, ExecAgentAuthMode::CorivoProxy);
    }

    #[test]
    fn legacy_system_claude_aliases_to_corivo_proxy() {
        // SystemClaude path was deleted entirely; the closest survivor
        // is CorivoProxy — at least the app boots, user can switch to
        // BYOK in Settings if they relied on system claude credentials.
        let cfg = load_with_legacy("system_claude");
        assert_eq!(cfg.exec_agent.auth_mode, ExecAgentAuthMode::CorivoProxy);
    }

    #[test]
    fn legacy_api_key_aliases_to_byok() {
        let cfg = load_with_legacy("api_key");
        assert_eq!(cfg.exec_agent.auth_mode, ExecAgentAuthMode::Byok);
    }

    #[test]
    fn aliased_value_rewrites_to_canonical_on_save() {
        // After load, the backend should have been re-saved with the
        // canonical variant name — aliases are self-cleaning.
        let backend = Arc::new(InMemoryConfigStore::default());
        backend
            .set_json(CONFIG_KEY, legacy_config_json("corivo_login"))
            .unwrap();
        let _svc = ConfigService::with_backend(backend.clone()).unwrap();

        let saved = backend
            .snapshot_value(CONFIG_KEY)
            .expect("config should have been saved on load");
        let auth_mode = saved
            .pointer("/exec_agent/auth_mode")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(auth_mode, "corivo_proxy");
    }

    #[test]
    fn legacy_connectors_shape_resets_to_default() {
        // Pre-3ffab0a: `accounts` keyed by connector_id (no ":"), and
        // `bindings` absent entirely. After load, the whole connectors
        // block should be wiped — keychain naming changed in the same
        // commit, so the old tokens are dead anyway.
        let backend = Arc::new(InMemoryConfigStore::default());
        backend
            .set_json(
                CONFIG_KEY,
                serde_json::json!({
                    "connectors": {
                        "enabled": ["gmail"],
                        "accounts": {
                            "gmail": {
                                "provider": "google",
                                "accountId": "117625",
                                "email": "test@example.com",
                                "displayName": null,
                                "avatarUrl": null,
                                "grantedScopes": ["openid"],
                                "connectedAt": "2026-05-12T06:41:38Z",
                                "lastRefreshAt": null,
                                "needsReauth": false
                            }
                        }
                    }
                }),
            )
            .unwrap();
        let svc = ConfigService::with_backend(backend).expect("must not panic");
        let cfg = svc.get();
        assert!(cfg.connectors.accounts.is_empty());
        assert!(cfg.connectors.bindings.is_empty());
        assert!(cfg.connectors.enabled.is_empty());
    }

    #[test]
    fn new_connectors_shape_preserved() {
        // Post-3ffab0a: account keys contain ":", bindings populated.
        // The migration must leave this alone.
        let backend = Arc::new(InMemoryConfigStore::default());
        backend
            .set_json(
                CONFIG_KEY,
                serde_json::json!({
                    "connectors": {
                        "enabled": ["gmail"],
                        "bindings": { "gmail": "google:117625" },
                        "accounts": {
                            "google:117625": {
                                "provider": "google",
                                "accountId": "117625",
                                "email": "test@example.com",
                                "displayName": null,
                                "avatarUrl": null,
                                "grantedScopes": ["openid"],
                                "connectedAt": "2026-05-12T06:41:38Z",
                                "lastRefreshAt": null,
                                "needsReauth": false
                            }
                        }
                    }
                }),
            )
            .unwrap();
        let svc = ConfigService::with_backend(backend).expect("must not panic");
        let cfg = svc.get();
        assert_eq!(cfg.connectors.enabled, vec!["gmail".to_string()]);
        assert!(cfg.connectors.accounts.contains_key("google:117625"));
        assert_eq!(
            cfg.connectors.bindings.get("gmail"),
            Some(&"google:117625".to_string())
        );
    }

    #[test]
    fn unparseable_config_falls_back_to_default_without_panic() {
        // Defense-in-depth: even if a future schema change escapes serde
        // aliases entirely, we don't panic — we reset to default and
        // surface a tracing::error. The app must always boot.
        let backend = Arc::new(InMemoryConfigStore::default());
        backend
            .set_json(
                CONFIG_KEY,
                serde_json::json!({
                    "exec_agent": {
                        "auth_mode": "completely_unknown_variant_from_the_future"
                    }
                }),
            )
            .unwrap();
        let svc = ConfigService::with_backend(backend).expect("must not panic");
        // Falls back to the OSS default Config (auth_mode = Byok).
        assert_eq!(svc.get().exec_agent.auth_mode, ExecAgentAuthMode::Byok);
    }
}
