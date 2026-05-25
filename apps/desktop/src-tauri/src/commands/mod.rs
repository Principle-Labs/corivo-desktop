pub mod analytics;
pub mod app_icon;
pub mod auth;
pub mod billing;
pub mod capture;
pub mod chat;
pub mod cloud;
// Composio gateway commands talk to `${API_BASE}/composio/...` directly
// and have no offline counterpart. Closed Corivo build only; OSS hides
// the consuming UI via the `connectors` capability flag and the
// invoke_handler! registrations below are cfg-gated to match.
#[cfg(feature = "corivo-cloud")]
pub mod composio;
pub mod config;
pub mod connectors;
pub mod dev;
pub mod exec_agent;
pub mod frames;
pub mod log;
pub mod memory;
pub mod models;
pub mod onboarding;
pub mod privacy;
pub mod quick_ask;
pub mod settings;
pub mod skills;
pub mod workflows;
