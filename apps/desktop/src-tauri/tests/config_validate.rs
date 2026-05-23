use corivo_app_lib::domain::config::{Config, ConfigError};

#[test]
fn default_config_is_valid() {
    Config::default().validate().expect("default must validate");
}

#[test]
fn negative_confidence_weight_is_rejected() {
    let mut cfg = Config::default();
    cfg.user_model.retrieval.w_confidence = -0.5;
    assert!(matches!(
        cfg.validate(),
        Err(ConfigError::Negative {
            field: "user_model.retrieval.w_confidence",
            ..
        })
    ));
}

#[test]
fn negative_decay_weight_is_rejected() {
    let mut cfg = Config::default();
    cfg.user_model.retrieval.w_decay = -1.0;
    assert!(matches!(
        cfg.validate(),
        Err(ConfigError::Negative {
            field: "user_model.retrieval.w_decay",
            ..
        })
    ));
}

#[test]
fn k_decay_days_must_be_positive() {
    let mut cfg = Config::default();
    cfg.user_model.retrieval.k_decay_days = 0.0;
    assert!(matches!(
        cfg.validate(),
        Err(ConfigError::Negative {
            field: "user_model.retrieval.k_decay_days",
            ..
        })
    ));
}

#[test]
fn batcher_min_batch_size_zero_is_rejected() {
    let mut cfg = Config::default();
    cfg.user_model.batcher.min_batch_size = 0;
    assert!(matches!(
        cfg.validate(),
        Err(ConfigError::NonPositive {
            field: "user_model.batcher.min_batch_size",
            ..
        })
    ));
}

#[test]
fn prompt_debug_config_default_has_suggest_override_none() {
    use corivo_app_lib::domain::config::PromptDebugConfig;
    let cfg = PromptDebugConfig::default();
    assert!(cfg.suggest_override.is_none());
}

#[test]
fn prompt_debug_config_normalize_blank_suggest_override_to_none() {
    let mut cfg = Config::default();
    cfg.prompt_debug.suggest_override = Some("   \n  ".to_string());
    let normalized = cfg.normalize();
    assert!(normalized.prompt_debug.suggest_override.is_none());
}

#[test]
fn prompt_debug_config_default_has_score_override_none() {
    use corivo_app_lib::domain::config::PromptDebugConfig;
    let cfg = PromptDebugConfig::default();
    assert!(cfg.score_override.is_none());
}

#[test]
fn prompt_debug_config_normalize_blank_score_override_to_none() {
    let mut cfg = Config::default();
    cfg.prompt_debug.score_override = Some("   \n  ".to_string());
    let normalized = cfg.normalize();
    assert!(normalized.prompt_debug.score_override.is_none());
}
