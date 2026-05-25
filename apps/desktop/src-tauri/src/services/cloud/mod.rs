//! Cloud capability layer.
//!
//! The desktop ships in two flavours:
//!
//! * **Open-source build (default).** Pure-local. The user runs Corivo
//!   without an account, brings their own Anthropic API key, and the
//!   updater is not registered. Every cloud capability resolves to a
//!   [`noop`] implementation that returns
//!   [`CorivoError::FeatureUnavailable`].
//! * **Closed Corivo build (`--features corivo-cloud`).** The same crate,
//!   plus implementations that talk to the closed Corivo backend: Google + email
//!   sign-in, Stripe top-ups, a managed model directory, third-party
//!   connectors (Composio), Sentry telemetry, and the corivo-policy
//!   updater. Lives under `cloud::corivo`.
//!
//! Everything routes through trait objects on [`CloudServices`], so
//! command handlers never branch on "is the cloud wired?" — they call
//! the trait method and let the noop impl produce the right error /
//! fallback. This keeps the IPC surface identical between builds; the
//! frontend asks `get_capabilities` once at boot and hides UI for the
//! ones it can't actually use.
//!
//! ### Adding a capability
//!
//! 1. Add a `trait XxxService` here, with `#[async_trait]` methods that
//!    return `crate::error::Result<T>`.
//! 2. Add a `NoopXxxService` in [`noop`] that returns
//!    `CorivoError::FeatureUnavailable { feature: "xxx", … }` from every
//!    method (or a sensible empty / local-only value when the operation
//!    has a meaningful offline answer).
//! 3. Behind `#[cfg(feature = "corivo-cloud")]`, add the real impl in
//!    `cloud::corivo::xxx`.
//! 4. Add the field to [`CloudServices`] and update the assembly fns.
//! 5. Mirror the capability bit in `commands::cloud::get_capabilities`.

pub mod auth;
pub mod billing;
pub mod connectors;
pub mod models;
pub mod session;
pub mod telemetry;
pub mod updater_policy;

pub mod noop;
#[cfg(feature = "corivo-cloud")]
pub(crate) mod private_hooks;

// The closed-source overlay (`cloud::corivo`) ships only in the
// private Corivo monorepo and is materialized into this directory by
// `corivo-app/scripts/prepare-submodule.mjs` when building under the
// `corivo-cloud` cargo feature. OSS builds leave this feature off, so
// the module reference is dead code and the directory does not need
// to exist — see ../../../../README.md for how the trait split works.
//
// Forks that want a different cloud backend can add their own sibling
// module behind a different feature and swap individual fields on
// `CloudServices` during boot.
#[cfg(feature = "corivo-cloud")]
pub mod corivo;

use std::sync::Arc;

pub use auth::AuthService;
pub use billing::BillingService;
pub use connectors::ConnectorsService;
pub use models::ModelsService;
pub use session::CloudSessionService;
pub use telemetry::TelemetryService;
pub use updater_policy::UpdaterPolicyService;

// NOTE on agent credentials: the corivo-agent sidecar already has a
// well-shaped abstraction in `services::exec_agent::runner::CorivoAuth`
// (a `CorivoProxy { gateway_url, api_key } | Byok { base_url, api_key }`
// enum) driven by `Config.exec_agent.auth_mode`. Cloud-build users
// stay on `CorivoProxy`; the open-source build flips the default to
// `Byok` via the cfg-gated `Default` impl on `ExecAgentAuthMode`
// (see `domain::config`). A separate `AgentCredsProvider` trait here
// would just shadow that enum without adding capability — deliberately
// not introduced.

/// The bundle of cloud-capability trait objects mounted on `AppState`.
///
/// Always non-null. When a capability isn't compiled into this build
/// (or hasn't been wired at boot for some other reason), the matching
/// field holds a noop impl that produces
/// [`crate::error::CorivoError::FeatureUnavailable`] from every method
/// — command handlers never have to test for "not configured" by hand.
#[derive(Clone)]
pub struct CloudServices {
    pub auth: Arc<dyn AuthService>,
    pub billing: Arc<dyn BillingService>,
    pub models: Arc<dyn ModelsService>,
    pub session: Arc<dyn CloudSessionService>,
    pub connectors: Arc<dyn ConnectorsService>,
    pub telemetry: Arc<dyn TelemetryService>,
    pub updater_policy: Arc<dyn UpdaterPolicyService>,
}

impl CloudServices {
    /// All-noop bundle. Open-source builds use this verbatim; closed
    /// builds use it as a starting point and swap individual fields
    /// from `cloud::corivo` once their dependencies (config service,
    /// app handle, …) are available.
    pub fn noop() -> Self {
        Self {
            auth: Arc::new(noop::NoopAuthService),
            billing: Arc::new(noop::NoopBillingService),
            models: Arc::new(noop::NoopModelsService),
            session: Arc::new(noop::NoopCloudSessionService),
            connectors: Arc::new(noop::NoopConnectorsService),
            telemetry: Arc::new(noop::NoopTelemetryService),
            updater_policy: Arc::new(noop::NoopUpdaterPolicyService),
        }
    }

    /// Capability snapshot exposed to the frontend. `true` means the
    /// trait object is a real implementation; `false` means it's a
    /// noop and the UI should hide the corresponding entry points.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            auth: self.auth.is_available(),
            billing: self.billing.is_available(),
            models_directory: self.models.is_available(),
            connectors: self.connectors.is_available(),
            telemetry: self.telemetry.is_available(),
            managed_updater: self.updater_policy.is_managed(),
        }
    }
}

/// Wire-level shape returned by the `get_capabilities` command. Mirror
/// of the booleans the trait objects advertise about themselves. Kept
/// flat (no `Option`s, no nesting) because the frontend treats every
/// missing field as "off" and we want adding a capability to be a
/// purely additive change for old desktop builds talking to a future
/// shared-types release.
#[derive(Debug, Clone, Copy, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub auth: bool,
    pub billing: bool,
    pub models_directory: bool,
    pub connectors: bool,
    pub telemetry: bool,
    pub managed_updater: bool,
}
