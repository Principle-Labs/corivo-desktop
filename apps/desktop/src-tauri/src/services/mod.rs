pub mod background_agent_task;
pub mod capture_client;
pub mod capture_pipeline;
pub mod capture_store;
pub mod chatgpt_auth;
// Cloud capability trait layer. Trait objects + noop defaults live here;
// real Corivo implementations are materialized from private-src once the
// `corivo-cloud` cargo feature is enabled. See `cloud::mod` for the
// migration shape.
pub mod cloud;
pub mod config_service;
pub mod connector;
#[cfg(feature = "corivo-cloud")]
pub mod corivo_session;
pub mod double_tap_hotkey;
pub mod exclusion;
pub mod exec_agent;
pub mod extractor;
pub mod foreground_monitor;
pub mod hotkey;
pub mod macos_system_surface;
pub mod memory;
pub mod model_catalog;
pub mod oauth_loopback;
pub mod persona;
pub mod privacy_filter;
pub mod quick_ask_window;
pub mod recall;
pub mod retention;
pub mod scheduled_workflows;
pub mod session_learner;
pub mod skill_share;
pub mod snapshot_consumer;
pub mod tokenize;
pub mod website_exclusion;
