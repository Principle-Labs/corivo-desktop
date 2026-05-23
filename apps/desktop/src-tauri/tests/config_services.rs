use std::sync::Arc;

use corivo_app_lib::{
    domain::config::{Config, Language},
    services::config_service::{ConfigService, ConfigStoreBackend, InMemoryConfigStore},
};

#[test]
fn config_service_initializes_defaults_and_persists_updates() {
    let store = Arc::new(InMemoryConfigStore::default());
    let service = ConfigService::with_backend(store.clone()).expect("config service");

    let config = service.get();
    assert_eq!(config.capture.interval_secs, 60);
    assert_eq!(config.capture.batch_size, 5);
    assert_eq!(config.capture.jpeg_quality, 75);
    // First-run path: `load_from_store` saw no `config.json` and seeded
    // both language fields from the host's OS locale. The mapping
    // collapses to Zh ↔ En, so we compare against the same detector
    // the production code calls.
    let detected = Language::detect_from_os();
    assert_eq!(config.app.ui_language, detected);
    assert_eq!(config.app.response_language, detected);
    // start_capture_on_launch defaults to true so capture begins on first
    // boot without requiring the onboarding gate to flip it.
    assert!(config.app.start_capture_on_launch);
    assert!(!config.app.onboarding_completed);
    assert_eq!(config.app.onboarding_version, 0);
    assert_eq!(config.prompt_debug.summary_override, None);
    assert_eq!(config.prompt_debug.push_judgment_override, None);
    // Session tokens default to absent on a fresh install.
    assert_eq!(config.exec_agent.byok_key, None);
    assert_eq!(config.corivo_session.access_token, None);
    assert_eq!(config.corivo_session.access_expires_at, None);
    assert_eq!(config.corivo_session.refresh_token, None);
    assert_eq!(config.corivo_session.refresh_expires_at, None);

    let mut updated = config.clone();
    updated.app.start_capture_on_launch = true;
    updated.app.onboarding_completed = true;
    updated.app.onboarding_version = 1;
    updated.exec_agent.byok_key = Some("sk-ant-test".into());
    updated.corivo_session.access_token = Some("access-token".into());
    updated.corivo_session.access_expires_at = Some("2099-01-01T00:00:00Z".into());
    updated.corivo_session.refresh_token = Some("refresh-token".into());
    updated.corivo_session.refresh_expires_at = Some("2099-01-31T00:00:00Z".into());

    service.update(updated.clone()).expect("update config");

    let after = service.get();
    assert!(after.app.start_capture_on_launch);
    assert!(after.app.onboarding_completed);
    assert_eq!(after.app.onboarding_version, 1);
    assert_eq!(after.prompt_debug.summary_override, None);
    assert_eq!(after.prompt_debug.push_judgment_override, None);
    assert_eq!(after.exec_agent.byok_key.as_deref(), Some("sk-ant-test"));
    assert_eq!(
        after.corivo_session.access_token.as_deref(),
        Some("access-token")
    );
    assert_eq!(
        after.corivo_session.refresh_token.as_deref(),
        Some("refresh-token")
    );

    // Secrets actually round-trip through the on-disk JSON snapshot.
    let serialized = store.snapshot_value("config").expect("config snapshot");
    let serialized_text = serialized.to_string();
    assert!(serialized_text.contains("sk-ant-test"));
    assert!(serialized_text.contains("access-token"));
    assert!(serialized_text.contains("refresh-token"));
}

#[test]
fn config_service_migrates_legacy_capture_field_names_and_drops_removed_blocks() {
    // The on-disk JSON predates the Stage 2 cleanup, so it carries
    // `summary`, `notification`, `app.auto_start`, `app.theme`,
    // `app.notifications_enabled`, and the legacy
    // `capture.segment_duration_mins`. Serde drops every unknown key
    // because each struct is `#[serde(default)]`, and `batch_size`
    // picks up the legacy alias.
    let store = Arc::new(InMemoryConfigStore::default());
    store
        .set_json(
            "config",
            serde_json::json!({
                "capture": {
                    "interval_secs": 20,
                    "segment_duration_mins": 9,
                    "max_storage_gb": 7
                },
                "summary": {
                    "prompt_template": "prompt",
                    "model": "gemini-3-flash-preview"
                },
                "app": {
                    "auto_start": false,
                    "minimize_to_tray": true,
                    "notifications_enabled": true,
                    "theme": "system"
                }
            }),
        )
        .expect("seed legacy config");

    let service = ConfigService::with_backend(store).expect("config service");
    let config = service.get();

    assert_eq!(config.capture.batch_size, 9);
    assert_eq!(config.capture.jpeg_quality, 75);
    // Legacy JSON path: existing config doesn't carry the language
    // fields, so serde fills them from `Language::default()` (English).
    // No OS-locale seeding here — that only runs on first install.
    assert_eq!(config.app.ui_language, Language::En);
    assert_eq!(config.app.response_language, Language::En);
    assert!(config.app.minimize_to_tray);
    // start_capture_on_launch defaults to true so capture begins on first
    // boot without requiring the onboarding gate to flip it.
    assert!(config.app.start_capture_on_launch);
    assert!(!config.app.onboarding_completed);
    assert_eq!(config.app.onboarding_version, 0);
    assert_eq!(config.prompt_debug.summary_override, None);
    assert_eq!(config.prompt_debug.push_judgment_override, None);
}

#[test]
fn user_model_config_defaults_round_trip_through_json() {
    // Pins spec §5.Config defaults for the GUM user model. Future PRs must
    // not silently rename, retype, or drop any of these fields without
    // consciously updating this test.
    let config = Config::default();

    let serialized = serde_json::to_string(&config).expect("serialize Config::default");
    let deserialized: Config =
        serde_json::from_str(&serialized).expect("deserialize Config default");

    assert_eq!(deserialized.user_model.batcher.min_batch_size, 5);
    assert_eq!(deserialized.user_model.batcher.flush_interval_ms, 30_000);

    assert_eq!(deserialized.user_model.retrieval.w_confidence, 1.0);
    assert_eq!(deserialized.user_model.retrieval.w_decay, 1.0);
    assert_eq!(deserialized.user_model.retrieval.k_decay_days, 14.0);
    assert_eq!(deserialized.user_model.retrieval.limit_multiplier, 3);

    assert_eq!(deserialized.user_model.pipeline.similar_pool_size, 20);

    // Round-trip must be a fixed point.
    assert_eq!(deserialized, config);
}

#[test]
fn config_service_defaults_prompt_debug_overrides_when_missing_from_store() {
    let store = Arc::new(InMemoryConfigStore::default());
    store
        .set_json(
            "config",
            serde_json::json!({
                "capture": {
                    "interval_secs": 30,
                    "batch_size": 5,
                    "max_storage_gb": 5,
                    "jpeg_quality": 75
                },
                "app": {
                    "minimize_to_tray": true,
                    "language": "zh",
                    "start_capture_on_launch": false,
                    "onboarding_completed": false,
                    "onboarding_version": 0
                }
            }),
        )
        .expect("seed config without prompt_debug");

    let service = ConfigService::with_backend(store).expect("config service");
    let config = service.get();

    assert_eq!(config.prompt_debug.summary_override, None);
    assert_eq!(config.prompt_debug.push_judgment_override, None);
}

#[test]
fn config_service_normalizes_blank_prompt_debug_overrides_to_none() {
    let store = Arc::new(InMemoryConfigStore::default());
    let service = ConfigService::with_backend(store).expect("config service");

    let mut updated = service.get();
    updated.prompt_debug.summary_override = Some("".to_string());
    updated.prompt_debug.push_judgment_override = Some("   ".to_string());

    service.update(updated).expect("update config");

    let config = service.get();
    assert_eq!(config.prompt_debug.summary_override, None);
    assert_eq!(config.prompt_debug.push_judgment_override, None);
}
