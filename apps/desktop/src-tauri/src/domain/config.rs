use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

const DEFAULT_CAPTURE_BROADCAST_CAPACITY: u32 = 64;

/// Globally-broadcast event payload for `config:changed`. Emitted from
/// `set_config` so every webview (main + Quick Ask) can invalidate its
/// React Query `["config"]` cache and re-fetch the canonical state.
///
/// Same shape as `ChatThreadsChanged` — Tauri webviews are independent
/// JS runtimes with separate `QueryClient` instances, so a one-window
/// invalidate alone leaves the others stale. The Quick Ask overlay
/// reads `Config.app.ui_language` for its `I18nProvider`, so a stale
/// cache here surfaces directly as "language flip didn't take effect".
pub struct ConfigChanged;

impl ConfigChanged {
    pub const EVENT: &'static str = "config:changed";
}

// UserModelConfig defaults (PR1 skeleton; real wiring lands in PR2+).
// Values anchored to spec §7.3 / §7.5 / §13.4 so defaults match the contract
// this refactor will implement; no magic numbers scattered across services.
const DEFAULT_BATCHER_MIN_BATCH_SIZE: u32 = 5;
const DEFAULT_BATCHER_FLUSH_INTERVAL_MS: u32 = 30_000;
const DEFAULT_RETRIEVAL_W_CONFIDENCE: f32 = 1.0;
const DEFAULT_RETRIEVAL_W_DECAY: f32 = 1.0;
const DEFAULT_RETRIEVAL_K_DECAY_DAYS: f32 = 14.0;
const DEFAULT_RETRIEVAL_LIMIT_MULTIPLIER: u32 = 3;
const DEFAULT_PIPELINE_SIMILAR_POOL_SIZE: u32 = 20;
const DEFAULT_PIPELINE_MIN_DRAFT_COUNT: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub capture: CaptureConfig,
    pub app: AppConfig,
    pub prompt_debug: PromptDebugConfig,
    pub user_model: UserModelConfig,
    pub soul: SoulConfig,
    /// Daily Project reconcile (docs/project-reconcile-spec.md §9).
    pub reconcile: ReconcileConfig,
    /// Focus-session layer that bundles sequential WorkContexts for
    /// cheaper + more context-aware attribution.
    pub focus_session: FocusSessionConfig,
    /// Privacy exclusion list — appended to the curated default
    /// blocklist when the capture pipeline runs the foreground app
    /// through the exclusion engine (spec §六).
    pub exclusion: ExclusionConfig,
    /// Executive agent (corivo-agent sidecar) — picks how the sidecar
    /// authenticates upstream and which model it talks to.
    pub exec_agent: ExecAgentConfig,
    /// Closed-beta Corivo session token pair. Persisted as plaintext in
    /// `config.json` alongside everything else.
    pub corivo_session: CorivoSessionConfig,
    /// External-service connector framework. Only NON-secret metadata
    /// (email / display_name / granted scopes / connected_at …) lives
    /// here. The actual OAuth `access_token` + `refresh_token` are in
    /// the macOS Keychain — see `services::connector::secrets`.
    pub connectors: ConnectorsConfig,
    /// Privacy filter (docs/privacy-filter-spec.md) —— OpenAI privacy-filter
    /// 本地 PII 检测的用户偏好。默认 `enabled = false`,直到用户在
    /// Settings 同意下载模型 (~830MB) 后才打开。`secret` 类目永远强制
    /// 启用 (spec §12.1)。
    pub privacy_filter: crate::domain::privacy::PrivacySettings,
}

/// Default model id for fresh installs. Matches the first `tier=main`
/// row in `services::model_catalog::seeded_demo_models` and the
/// canonical recommendation against the closed-beta gateway. The
/// Settings UI in `exec-agent-section.tsx` auto-falls-back to the
/// first live `tier=main` model if `/v1/models` ever drops this id.
pub const DEFAULT_SELECTED_MODEL_ID: &str = "claude-sonnet-4-6";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecAgentConfig {
    /// How the corivo-agent sidecar authenticates upstream
    /// (spec §7.3 / §7.4).
    pub auth_mode: ExecAgentAuthMode,
    /// CorivoProxy-mode pick: id of the model the user selected from
    /// the §7.5 cached `/v1/models` list. Sidecar resolves the API
    /// shape + compaction partner via that cache. Defaults to
    /// `DEFAULT_SELECTED_MODEL_ID` so a fresh install can chat without
    /// visiting Settings first.
    pub selected_model_id: Option<String>,
    /// Byok-mode: which provider protocol to talk. Drives the request
    /// shape pi-ai uses inside the sidecar.
    pub byok_api_shape: Option<ApiShape>,
    /// Byok-mode: optional override for the provider base URL. `None`
    /// falls back to the SDK default (Anthropic / OpenAI public).
    pub byok_base_url: Option<String>,
    /// Byok-mode: user-supplied API key.
    pub byok_key: Option<String>,
    /// Byok-mode: user-supplied model id (e.g.
    /// `claude-sonnet-4-20250514`, `gpt-4o`). Sidecar validates against
    /// the provider's `/v1/models` on startup.
    pub byok_model: Option<String>,
    /// Reasoning budget the sidecar forwards on the main turn (spec
    /// §5.1 model.thinking_level). Defaults to `Medium` so power users
    /// see thinking output without configuring.
    pub thinking_level: ThinkingLevel,
    /// Bridge selected host-side skills (~/.agents/skills/, ~/.claude/skills/)
    /// into the per-app skills dir via symlinks.
    pub skill_share: SkillShareConfig,
}

impl Default for ExecAgentConfig {
    fn default() -> Self {
        Self {
            auth_mode: ExecAgentAuthMode::default(),
            selected_model_id: Some(DEFAULT_SELECTED_MODEL_ID.to_string()),
            byok_api_shape: None,
            byok_base_url: None,
            byok_key: None,
            byok_model: None,
            thinking_level: ThinkingLevel::default(),
            skill_share: SkillShareConfig::default(),
        }
    }
}

/// Protocol shapes pi-ai routes through. Forwarded directly to the
/// sidecar via §5.1 `model.api_shape` / `compaction_model.api_shape`.
///
/// `Openai` 走 pi-ai 的 `openai-completions`（`POST /v1/chat/completions`，
/// `tool_calls[].id` 风格）。`OpenaiResponses` 走 pi-ai 的
/// `openai-responses`（`POST /v1/responses`，`function_call.call_id`
/// 风格）—— GPT-5 系列在 OpenAI 端只在 Responses API 才能稳定使用
/// 工具调用，所以这些 model 必须用这个分支。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ApiShape {
    Anthropic,
    Openai,
    OpenaiResponses,
}

impl ApiShape {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::OpenaiResponses => "openai_responses",
        }
    }
}

/// Thinking budget the sidecar forwards to pi-ai (`AgentState.thinkingLevel`).
/// `Off` ↔ no thinking; `Minimal..XHigh` map straight to pi-ai's enum.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    /// Matches pi-ai's "xhigh" wire value (no underscore) — overrides
    /// `rename_all = "snake_case"` for this single variant.
    #[serde(rename = "xhigh")]
    #[ts(rename = "xhigh")]
    XHigh,
}

impl ThinkingLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

/// Closed-beta session bookkeeping. Holds the OAuth-style access /
/// refresh token pair managed by the cloud auth implementation at
/// login and refresh time.
///
/// - `access_token` is short-lived (1h, server-controlled); the
///   desktop sends it as the bearer on every protected request and
///   trades the refresh token for a new pair on 401 or local expiry.
/// - `refresh_token` is long-lived (30d, sliding); kept on disk so
///   surviving an app restart doesn't force the user back through
///   Google sign-in.
///
/// All four fields are `None` together when the user is signed out.
/// `*_expires_at` are RFC3339 strings — same shape the server returns,
/// stored verbatim to avoid timezone games.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CorivoSessionConfig {
    pub access_token: Option<String>,
    #[serde(rename = "accessExpiresAt")]
    pub access_expires_at: Option<String>,
    pub refresh_token: Option<String>,
    #[serde(rename = "refreshExpiresAt")]
    pub refresh_expires_at: Option<String>,
}

/// External-service connector state. Each user can enable any number of
/// connectors from the built-in catalog; multiple connectors can share
/// a single upstream account (e.g. gcal + gmail + google-docs all
/// pointing at the same Google account → one OAuth, one refresh token).
///
/// Two-level model:
/// * `accounts` — provider-level identity rows, keyed by
///   `"<provider>:<account_id>"` (e.g. `"google:117625…"`). Holds the
///   union of scopes granted across every connector bound to it,
///   PLUS the access / refresh tokens for that account.
/// * `bindings` — per-connector pointer into `accounts`. Multiple
///   connector ids may map to the same account key; disconnecting one
///   connector just removes its binding, and only when the last
///   binding for an account is gone do we evict the account row +
///   its tokens.
///
/// Tokens live here in `config.json` (plaintext, via `tauri-plugin-store`),
/// alongside the Anthropic API key and Corivo session token — same
/// trust model as the rest of `Config`. We deliberately do NOT use
/// macOS Keychain: the code-signature-bound ACL re-prompts the user on
/// every dev rebuild (the binary hash changes), and even in release
/// the marginal security gain over `config.json` doesn't justify the
/// "this app wants to access your keychain" friction. Google refresh
/// tokens can be revoked at <https://myaccount.google.com/permissions>
/// if the file ever leaks.
///
/// `mcpServer`-shape connectors (no OAuth on host side; mcporter owns
/// the cache) also flow through this model with `provider = "mcp"` and
/// `account_id = <connector_id>`, so the data shape stays uniform. For
/// those the token fields stay `None` — mcporter persists its own
/// tokens to disk under `$APPDATA/corivo-mcp-tokens/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectorsConfig {
    /// Connector ids the user has explicitly enabled. Order is the user's
    /// arrangement; UI renders in this order.
    pub enabled: Vec<String>,
    /// `"<provider>:<account_id>"` → account metadata. Entry presence ≠
    /// "actively used" — `needs_reauth=true` means refresh_token died,
    /// the row stays so we keep the email shown in the "reconnect"
    /// prompt.
    pub accounts: HashMap<String, ProviderAccount>,
    /// connector_id → `"<provider>:<account_id>"`. Reverse index telling
    /// the runtime which account a given connector borrows.
    pub bindings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    /// Lowercase provider tag: `"google"`, `"slack"`, `"mcp"`. Used as
    /// the first half of the account key + as a provider router for
    /// OAuth client credentials / endpoint URLs.
    pub provider: String,
    /// Provider-side stable account id (Google `sub`, Slack
    /// `team_id:user_id`, mcpServer connector id). Survives email changes.
    pub account_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Union of scopes the IdP has granted across every connector bound
    /// to this account. Grows as users connect additional connectors
    /// (Google's `include_granted_scopes=true` makes this monotonic).
    pub granted_scopes: Vec<String>,
    /// ISO-8601.
    pub connected_at: String,
    /// ISO-8601. None until the first refresh round-trip.
    pub last_refresh_at: Option<String>,
    /// Set to true when the refresh_token has been rejected
    /// (`invalid_grant`) — the UI surfaces a red "reconnect" state.
    pub needs_reauth: bool,
    /// Current OAuth bearer. May be expired — check `expires_at` /
    /// refresh via `refresh_token`. `None` for mcpServer accounts
    /// (mcporter owns those tokens on disk).
    #[serde(default)]
    pub access_token: Option<String>,
    /// Long-lived OAuth refresh credential. Plaintext in `config.json`
    /// — see the [`ConnectorsConfig`] docstring for why we don't use
    /// Keychain.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC3339 expiry of `access_token`. When `Utc::now() + leeway >=
    /// expires_at`, the auth layer transparently refreshes before
    /// handing the token to a tool. `None` ↔ "IdP didn't tell us" →
    /// optimistically assume fresh, fall through to a 401 retry if
    /// the consumer cares.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Closed-beta skill-bridging policy. The runner spawns the bundled
/// `claude` against an isolated `CLAUDE_CONFIG_DIR`. On first launch
/// we seed `enabled` with every skill currently discovered on host
/// (see `lib.rs` boot block) so the user gets working tooling out of
/// the box; afterwards the Settings UI is the single source of truth.
/// `services::skill_share::sync` builds per-skill symlinks under
/// `$APPDATA/claude-config/skills/<name>` from whichever host source
/// provides that skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkillShareConfig {
    /// Skill names enabled for bridging. Names not currently present on
    /// host are kept (so re-installing a skill restores it without
    /// re-checking the box) but produce no symlink until the source
    /// reappears.
    pub enabled: Vec<String>,
    /// First-run latch. `false` on a fresh `config.json`; the boot
    /// sequence then fills `enabled` with every discovered skill and
    /// flips this to `true`. Subsequent launches respect whatever the
    /// user has in `enabled` — including an intentionally empty list.
    pub initialized: bool,
}

/// How the corivo-agent sidecar authenticates upstream (spec §7.3 / §7.4).
///
/// - `CorivoProxy` (closed-beta path): the user logged into Corivo;
///   the sidecar forwards the gateway base URL + bearer token. The
///   backend routes to the real provider behind the scenes. The cloud
///   session trait owns the credentials.
/// - `Byok`: user-supplied provider key (Anthropic / OpenAI / local
///   Ollama). The sidecar reads `byok_*` fields directly from
///   `ExecAgentConfig`.
///
/// ## Migration aliases
///
/// Pre-release we still want existing `config.json` files to keep working
/// across schema rewrites — losing a beta user's session_token because we
/// renamed an enum variant is a bad trade. The convention going forward:
/// **every rename / removal of a serde variant or field adds a `#[serde(alias = "old_name")]`
/// on whatever variant absorbs the legacy semantic.** On the next config
/// save the file is rewritten with the canonical name, so aliases are
/// self-cleaning.
///
/// Legacy variants currently aliased (Phase C §7.4 rename + SystemClaude
/// removal):
/// - `corivo_login` → `CorivoProxy` (pure rename)
/// - `system_claude` → `CorivoProxy` (Claude CLI path deleted; this is
///   the closest survivor — users who relied on `claude /login` settings
///   need to BYOK on next launch, but at least the app boots)
/// - `api_key` → `Byok` (pure rename)
///
/// ## Build-time default
///
/// Closed Corivo builds (cargo feature `corivo-cloud`) default to
/// `CorivoProxy` — that's the happy path for a paying beta user. The
/// open-source build flips the default to `Byok` because there's no
/// hosted gateway behind it; an OSS user with a fresh config would
/// otherwise hit a "no creds" error on first chat. See
/// `impl Default for ExecAgentAuthMode` below.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecAgentAuthMode {
    #[serde(alias = "corivo_login", alias = "system_claude")]
    CorivoProxy,
    #[serde(alias = "api_key")]
    Byok,
}

impl Default for ExecAgentAuthMode {
    // OSS default: BYOK (user pastes their own provider key) — fresh
    // installs land here so the first chat works without any cloud setup.
    // Closed builds default to CorivoProxy (the managed gateway). Both
    // variants remain valid runtime values either way, so a `config.json`
    // migrated from the other build shape still deserializes cleanly.
    fn default() -> Self {
        #[cfg(feature = "corivo-cloud")]
        {
            Self::CorivoProxy
        }
        #[cfg(not(feature = "corivo-cloud"))]
        {
            Self::Byok
        }
    }
}

impl ExecAgentAuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CorivoProxy => "corivo_proxy",
            Self::Byok => "byok",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExclusionConfig {
    /// Bundle ids the user has explicitly added on top of the default
    /// blocklist (signal / 1password / banking …). Stored verbatim;
    /// matched as exact strings.
    pub extra_app_bundle_ids: Vec<String>,
    /// Website host patterns the user excludes from context awareness.
    /// Format mirrors what the user types in the Settings UI:
    /// `notion.so` or `*.notion.so`. Parsed via
    /// `services::website_exclusion::WebsitePattern::parse` at engine
    /// build time; invalid entries are dropped silently on parse so a
    /// stale config never blocks app boot.
    pub extra_website_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ReconcileConfig {
    /// Master switch. When false the scheduler is still spawned but every
    /// tick exits before doing work — makes it safe to rollback via config
    /// without a redeploy.
    pub enabled: bool,
    /// Phase A (Project merge) sub-switch.
    pub merge_enabled: bool,
    /// Phase C (Stream clustering) sub-switch.
    pub stream_enabled: bool,
    /// Target cadence. Scheduler ticks more frequently and compares against
    /// `last_ran_at` from `reconcile_log`; this is the "don't run again
    /// until N hours have passed" gate, not a cron expression.
    pub interval_hours: u32,
    /// Projects with fewer than this many recent WorkContexts skip Stream
    /// clustering (§7.5 门槛).
    pub min_workcontexts_per_project_for_stream: u32,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            merge_enabled: true,
            stream_enabled: true,
            interval_hours: 24,
            min_workcontexts_per_project_for_stream: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FocusSessionConfig {
    /// Master switch. When false the pipeline skips session upsert and
    /// resolvers run the legacy per-WC path.
    pub enabled: bool,
    /// Minutes of inactivity between consecutive WC touches before a new
    /// session starts. Default 30 — matches a generous pomodoro break.
    pub gap_minutes: u32,
    /// Cutoff (in `gap_minutes` multiples) beyond which the idle-close
    /// sweep in reconcile phase B marks a session `closed`. Default 2×.
    pub idle_close_multiplier: u32,
    /// Upper bound on a single session's length. Past this we force-close
    /// even if WCs are still streaming in — prevents a monster "whole
    /// day" session that swallows everything. Default 4h.
    pub max_session_hours: u32,
}

impl Default for FocusSessionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gap_minutes: 30,
            idle_close_multiplier: 2,
            max_session_hours: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CaptureConfig {
    /// Safety-net cadence for the event-driven driver. The pipeline
    /// is event-driven (NSWorkspace + AXObserver + manual IPC); this
    /// only paces the periodic "I'm still here" tick that catches
    /// missed observers. Not surfaced in Settings — bumping this is
    /// a developer / debug knob.
    pub interval_secs: u64,
    #[serde(alias = "segment_duration_mins")]
    pub batch_size: u64,
    pub max_storage_gb: u64,
    pub jpeg_quality: u8,
    pub broadcast_capacity: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
    pub suggest_override: Option<String>,
    pub score_override: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoulPreset {
    #[default]
    Gentle,
    Accomplice,
    OldFriend,
    Coach,
    Custom,
}

impl SoulPreset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gentle => "gentle",
            Self::Accomplice => "accomplice",
            Self::OldFriend => "old_friend",
            Self::Coach => "coach",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SoulConfig {
    pub preset: SoulPreset,
    pub custom_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub minimize_to_tray: bool,
    /// Language Corivo's interface is rendered in. Drives the
    /// `I18nProvider` on the frontend; back-end logs / log lines stay
    /// English regardless. `serde` aliases keep older `config.json`
    /// files (which used `"language"`) working without a migration —
    /// pre-release projects still need read compat for in-flight devs.
    #[serde(alias = "language")]
    pub ui_language: Language,
    /// Language the LLM replies in for `/ask` and Quick Ask.
    /// Independent from `ui_language` so a Chinese-UI user can still
    /// get English answers (and vice versa). Injected as a brief
    /// directive on every chat turn — see
    /// [commands::exec_agent::exec_agent_send].
    pub response_language: Language,
    pub start_capture_on_launch: bool,
    pub onboarding_completed: bool,
    pub onboarding_version: u32,
    pub onboarding_step: Option<String>,
    /// Visual theme preference. `System` follows macOS appearance
    /// (default); `Light` / `Dark` lock it. The frontend's
    /// `useThemeSync` hook applies `[data-theme]` on the document root
    /// based on this value. Older `config.json` files where this field
    /// was a no-op string get coerced through serde's untagged fallback
    /// at deserialization (any unknown variant → `System`).
    pub theme: ThemePreference,
    /// Onboarding handoff flag — `false` until the user completes their
    /// very first Quick Ask turn during the Try-It step. Quick Ask reads
    /// this to decide whether to show the 3 suggestion chips and to fire
    /// the `onboarding:first-quick-ask` event so the onboarding window
    /// can dismiss itself. Once flipped to `true` it stays true for the
    /// life of the install — the chips never come back.
    /// See docs/design/auth-onboarding-v0.html § Asset Spec.
    #[serde(default)]
    pub first_quick_ask_done: bool,
    /// Wall-clock instant (RFC3339, UTC) when the menubar "Pause" timer
    /// expires. `None` means screen reading is not paused. Persisted so
    /// a quit-during-pause doesn't accidentally resume capture on next
    /// launch — `lib.rs::run` re-installs the auto-resume timer when it
    /// sees a future value, and clears stale past values.
    #[serde(default)]
    pub capture_paused_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Hidden "developer mode" toggle. Unlocked by triple-clicking the
    /// settings dialog title. When `true`, the settings sidebar surfaces
    /// a "Developer" section with WebView DevTools + on-device debugging
    /// shortcuts. The flag persists across launches; users disable it
    /// from inside the Developer section. Default false so first-run
    /// users see the regular settings only.
    #[serde(default)]
    pub developer_mode: bool,
}

/// Visual theme preference. Persisted in `Config.app.theme`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    Light,
    Dark,
    #[default]
    System,
}

impl ThemePreference {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Zh,
    En,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    /// Pick a sensible default language based on the host's primary
    /// system locale. Used at first launch (no `config.json` on disk
    /// yet) to greet the user in their own language instead of a hard
    /// English fallback. The mapping is deliberately coarse — `zh-*`
    /// (zh-CN / zh-TW / zh-Hant / etc.) → 简体中文, everything else →
    /// English. The login UI is the only English-or-Chinese-only
    /// surface today; richer locale selection lives in Settings →
    /// General.
    pub fn detect_from_os() -> Self {
        match sys_locale::get_locale() {
            Some(tag) if tag.to_ascii_lowercase().starts_with("zh") => Self::Zh,
            _ => Self::En,
        }
    }
}

/// Skeleton config tree for the GUM user model. PR1 lands the struct only;
/// PR2+ wires each sub-struct into the services that consume it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct UserModelConfig {
    pub batcher: BatcherConfig,
    pub retrieval: RetrievalConfig,
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BatcherConfig {
    pub min_batch_size: u32,
    pub flush_interval_ms: u32,
}

impl Default for BatcherConfig {
    fn default() -> Self {
        Self {
            min_batch_size: DEFAULT_BATCHER_MIN_BATCH_SIZE,
            flush_interval_ms: DEFAULT_BATCHER_FLUSH_INTERVAL_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetrievalConfig {
    pub w_confidence: f32,
    pub w_decay: f32,
    pub k_decay_days: f32,
    pub limit_multiplier: u32,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            w_confidence: DEFAULT_RETRIEVAL_W_CONFIDENCE,
            w_decay: DEFAULT_RETRIEVAL_W_DECAY,
            k_decay_days: DEFAULT_RETRIEVAL_K_DECAY_DAYS,
            limit_multiplier: DEFAULT_RETRIEVAL_LIMIT_MULTIPLIER,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PipelineConfig {
    pub similar_pool_size: u32,
    /// Lower bound on draft count PROPOSE must emit before the pipeline
    /// continues past that stage. `0` gets coerced to `1` at consumption.
    pub min_draft_count: u32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            similar_pool_size: DEFAULT_PIPELINE_SIMILAR_POOL_SIZE,
            min_draft_count: DEFAULT_PIPELINE_MIN_DRAFT_COUNT,
        }
    }
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            batch_size: 5,
            max_storage_gb: 5,
            jpeg_quality: 75,
            broadcast_capacity: DEFAULT_CAPTURE_BROADCAST_CAPACITY,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        // Language defaults are English here; the first-run path in
        // `config_service::load_from_store` overrides both fields with
        // `Language::detect_from_os()` before persisting, so a 中文 macOS
        // user still lands on a 中文 UI on the very first launch.
        Self {
            minimize_to_tray: true,
            ui_language: Language::En,
            response_language: Language::En,
            start_capture_on_launch: true,
            onboarding_completed: false,
            onboarding_version: 0,
            onboarding_step: None,
            theme: ThemePreference::default(),
            first_quick_ask_done: false,
            capture_paused_until: None,
            developer_mode: false,
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::En
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("{field} 必须大于 0，当前为 {value}")]
    NonPositive { field: &'static str, value: i64 },
    #[error("{field} 必须在 {min}..={max} 范围内，当前为 {value}")]
    OutOfRange {
        field: &'static str,
        min: i64,
        max: i64,
        value: i64,
    },
    #[error("{field} 必须非负，当前为 {value}")]
    Negative { field: &'static str, value: f32 },
}

impl Config {
    /// Validate the ranges the services downstream assume. Called by
    /// `config_service::load_from_store` so bad on-disk values fall back to
    /// `Config::default()` instead of breaking capture / push wiring.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.user_model.retrieval.w_confidence < 0.0 {
            return Err(ConfigError::Negative {
                field: "user_model.retrieval.w_confidence",
                value: self.user_model.retrieval.w_confidence,
            });
        }
        if self.user_model.retrieval.w_decay < 0.0 {
            return Err(ConfigError::Negative {
                field: "user_model.retrieval.w_decay",
                value: self.user_model.retrieval.w_decay,
            });
        }
        if self.user_model.retrieval.k_decay_days <= 0.0 {
            return Err(ConfigError::Negative {
                field: "user_model.retrieval.k_decay_days",
                value: self.user_model.retrieval.k_decay_days,
            });
        }
        if self.user_model.batcher.min_batch_size == 0 {
            return Err(ConfigError::NonPositive {
                field: "user_model.batcher.min_batch_size",
                value: 0,
            });
        }
        Ok(())
    }

    pub fn normalize(mut self) -> Self {
        self.prompt_debug.summary_override =
            normalize_optional_prompt_override(self.prompt_debug.summary_override);
        self.prompt_debug.push_judgment_override =
            normalize_optional_prompt_override(self.prompt_debug.push_judgment_override);
        self.prompt_debug.suggest_override =
            normalize_optional_prompt_override(self.prompt_debug.suggest_override);
        self.prompt_debug.score_override =
            normalize_optional_prompt_override(self.prompt_debug.score_override);
        // Trim custom SOUL text; treat whitespace-only as None; cap at 2000 chars
        // so a rogue paste doesn't balloon the prompt.
        self.soul.custom_text = self.soul.custom_text.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                let capped: String = trimmed.chars().take(2000).collect();
                Some(capped)
            }
        });
        // Custom preset without text → fall back to Gentle (same style as
        // other fallbacks here).
        if matches!(self.soul.preset, SoulPreset::Custom) && self.soul.custom_text.is_none() {
            self.soul.preset = SoulPreset::Gentle;
        }
        self
    }
}

fn normalize_optional_prompt_override(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    })
}

#[cfg(test)]
mod exec_agent_tests {
    use super::*;

    #[test]
    fn default_selected_model_is_sonnet_4_6() {
        let cfg = Config::default();
        assert_eq!(
            cfg.exec_agent.selected_model_id.as_deref(),
            Some(DEFAULT_SELECTED_MODEL_ID),
        );
    }

    #[test]
    fn missing_exec_agent_fields_deserialize_to_default() {
        let cfg: Config = serde_json::from_str("{}").expect("empty config should parse");
        assert_eq!(
            cfg.exec_agent.selected_model_id.as_deref(),
            Some(DEFAULT_SELECTED_MODEL_ID),
        );
    }

    #[test]
    fn explicit_null_selected_model_is_preserved() {
        let cfg: Config = serde_json::from_str(r#"{"exec_agent":{"selected_model_id":null}}"#)
            .expect("explicit null should parse");
        assert!(cfg.exec_agent.selected_model_id.is_none());
    }
}

#[cfg(test)]
mod soul_tests {
    use super::*;

    #[test]
    fn default_soul_is_gentle_with_no_custom_text() {
        let cfg = Config::default();
        assert_eq!(cfg.soul.preset, SoulPreset::Gentle);
        assert!(cfg.soul.custom_text.is_none());
    }

    #[test]
    fn normalize_trims_whitespace_only_custom_text_to_none() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some("   \n  \t".into());
        let cfg = cfg.normalize();
        assert!(cfg.soul.custom_text.is_none());
    }

    #[test]
    fn normalize_falls_back_to_gentle_when_custom_text_empty() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = None;
        let cfg = cfg.normalize();
        assert_eq!(cfg.soul.preset, SoulPreset::Gentle);
    }

    #[test]
    fn normalize_caps_custom_text_at_2000_chars() {
        let long = "测".repeat(3000);
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some(long);
        let cfg = cfg.normalize();
        let text = cfg.soul.custom_text.expect("custom text preserved");
        assert_eq!(text.chars().count(), 2000);
    }

    #[test]
    fn custom_preset_with_text_is_preserved() {
        let mut cfg = Config::default();
        cfg.soul.preset = SoulPreset::Custom;
        cfg.soul.custom_text = Some("你是独一无二的。".into());
        let cfg = cfg.normalize();
        assert_eq!(cfg.soul.preset, SoulPreset::Custom);
        assert_eq!(cfg.soul.custom_text.as_deref(), Some("你是独一无二的。"));
    }
}
