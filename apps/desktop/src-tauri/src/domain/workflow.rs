//! Scheduled-workflow domain types (v1510).
//!
//! Two sides of the same coin:
//!
//! * [`WorkflowDefinition`] — what the workflow *is*. Loaded from
//!   `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md` by
//!   `services::scheduled_workflows::store`. Frontmatter holds the
//!   metadata; the markdown body is the system-prompt template.
//!
//! * [`WorkflowSchedule`] / [`WorkflowRun`] — the runtime side. Stored
//!   in SQLite (`workflow_schedules` / `workflow_runs`). A schedule
//!   binds a [`Trigger`] to a slug; runs are immutable per-firing
//!   audit rows.
//!
//! [`Trigger`] is the only thing the UI lets the user pick. Each
//! variant gets its own `next_after` impl in
//! `services::scheduled_workflows::trigger`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Calendar trigger variants. v1 deliberately covers structured
/// pickers (`interval` / `daily` / `weekly` / `once`) plus a raw
/// `cron` escape hatch for power users.
///
/// Serialized as a tagged JSON union into
/// `workflow_schedules.trigger_expr`; `workflow_schedules.trigger_kind`
/// mirrors the discriminator for index-friendly queries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Every N minutes from the workflow's last firing (or its
    /// creation time on the very first run). The Ticker doesn't
    /// compensate for drift — a 15-minute trigger that takes 8 minutes
    /// to execute re-arms 15 minutes after `last_run_at`.
    Interval {
        #[ts(type = "number")]
        minutes: u32,
    },

    /// Daily at HH:MM in the given IANA timezone (e.g. `Asia/Shanghai`).
    Daily {
        #[ts(type = "number")]
        hour: u8,
        #[ts(type = "number")]
        minute: u8,
        tz: String,
    },

    /// Weekly on the given non-empty weekday set at HH:MM in the given
    /// IANA timezone.
    Weekly {
        weekdays: Vec<Weekday>,
        #[ts(type = "number")]
        hour: u8,
        #[ts(type = "number")]
        minute: u8,
        tz: String,
    },

    /// One-shot at a specific UTC instant. After firing, the Ticker
    /// auto-disables the schedule (sets `enabled = 0`).
    Once { at: DateTime<Utc> },

    /// Raw 5-field cron expression (POSIX-style: `min hour dom mon dow`),
    /// evaluated in `tz`. Internally normalized to 6-field for the
    /// `cron` crate by prepending a `0 ` seconds slot.
    Cron { expr: String, tz: String },
}

/// Day-of-week enum that ships across the IPC boundary. Maps to
/// `chrono::Weekday` for `next_after` math.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn to_chrono(self) -> chrono::Weekday {
        use chrono::Weekday as W;
        match self {
            Self::Mon => W::Mon,
            Self::Tue => W::Tue,
            Self::Wed => W::Wed,
            Self::Thu => W::Thu,
            Self::Fri => W::Fri,
            Self::Sat => W::Sat,
            Self::Sun => W::Sun,
        }
    }

    pub fn from_chrono(w: chrono::Weekday) -> Self {
        use chrono::Weekday as W;
        match w {
            W::Mon => Self::Mon,
            W::Tue => Self::Tue,
            W::Wed => Self::Wed,
            W::Thu => Self::Thu,
            W::Fri => Self::Fri,
            W::Sat => Self::Sat,
            W::Sun => Self::Sun,
        }
    }
}

/// Discriminator stored in `workflow_schedules.trigger_kind` — keeps
/// the index narrow and lets the Ticker filter on cheap text matches
/// instead of JSON parsing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Interval,
    Daily,
    Weekly,
    Once,
    Cron,
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Once => "once",
            Self::Cron => "cron",
        }
    }

    pub fn from_trigger(t: &Trigger) -> Self {
        match t {
            Trigger::Interval { .. } => Self::Interval,
            Trigger::Daily { .. } => Self::Daily,
            Trigger::Weekly { .. } => Self::Weekly,
            Trigger::Once { .. } => Self::Once,
            Trigger::Cron { .. } => Self::Cron,
        }
    }
}

/// One row of `workflow_schedules` — the runtime / lifecycle side of a
/// workflow. Definitions live separately in `WORKFLOW.md` files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct WorkflowSchedule {
    pub slug: String,
    pub trigger: Trigger,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<WorkflowRunStatus>,
    /// v1511 — who originally created this schedule. Preserved across
    /// user edits (the upsert path explicitly never overwrites it), so
    /// the UI badge stays accurate even after the user fine-tunes an
    /// agent-proposed schedule.
    pub source: WorkflowScheduleSource,
    /// v1511 — when `source == Agent`, the chat thread the agent was
    /// driving when it issued `schedule_task`. `None` for user-created
    /// schedules and for agent-created ones whose originating thread
    /// has been deleted (the FK is `ON DELETE SET NULL`).
    pub created_by_thread_id: Option<String>,
    /// v1512 — when to push a macOS banner + in-app toast after a run.
    /// Mirrored into the WORKFLOW.md frontmatter so the file is the
    /// source of truth; the DB column is just the cached value the
    /// Ticker / consume_output read at fire time.
    pub notify_policy: WorkflowNotifyPolicy,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// How loudly to surface a successful run (v1512). Failures notify
/// independently — a failed run is always interesting, regardless of
/// the policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS, Default)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotifyPolicy {
    /// Banner + toast every successful run. Default — most workflows
    /// the user explicitly creates exist *because* they want to see
    /// the result.
    #[default]
    Always,
    /// Banner + toast only when this run's `content_hash` differs from
    /// the previous run for this slug. Suppresses duplicate-output
    /// noise from recurring summaries.
    OnChange,
    /// Never push. Run still lands in `workflow_runs` and the sidebar
    /// "Corivo 提议" section (where the unread dot is the only signal).
    /// Right for fire-and-forget cleanup jobs and side-effect-only
    /// workflows.
    Silent,
}

impl WorkflowNotifyPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::OnChange => "on_change",
            Self::Silent => "silent",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "always" => Some(Self::Always),
            "on_change" => Some(Self::OnChange),
            "silent" => Some(Self::Silent),
            _ => None,
        }
    }
}

/// Who created a schedule. Stored in `workflow_schedules.source`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduleSource {
    /// Authored from the `/workflows` UI by the human.
    User,
    /// `corivo-agent` called the `schedule_task` native tool from
    /// inside a chat turn.
    Agent,
}

impl WorkflowScheduleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// Terminal status of one run. Stored on `workflow_runs.status` and
/// mirrored onto `workflow_schedules.last_status` after each firing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Success,
    Failure,
}

impl WorkflowRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

/// One row of `workflow_runs`. `thread_id` is `None` when the runner
/// failed before it could create the system chat thread (rare).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct WorkflowRun {
    pub id: String,
    pub slug: String,
    pub thread_id: Option<String>,
    pub status: WorkflowRunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub error_message: Option<String>,
    /// v1512 — short body text used by macOS banner / in-app toast /
    /// sidebar preview. `None` for legacy rows; populated going
    /// forward by `ScheduledWorkflowTask::consume_output`.
    pub summary: Option<String>,
    /// v1512 — sha256 of the raw assistant output. Used by
    /// `notify_policy = 'on_change'` to skip duplicate pushes.
    pub content_hash: Option<String>,
    /// v1512 — when the user opened / read this run via the sidebar
    /// "Corivo 提议" section or the workflow history dialog. `None`
    /// means unread; the count of unread rows drives the sidebar dot.
    pub acknowledged_at: Option<DateTime<Utc>>,
}

/// Frontmatter-derived definition loaded from
/// `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md`. The store rebuilds
/// this on every scan; consumers should treat it as cache.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct WorkflowDefinition {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    /// Native tool names the workflow is allowed to call. Intersected
    /// with the runtime registry by the background-agent runner.
    pub tool_whitelist: Vec<String>,
    #[ts(type = "number")]
    pub max_turns: u32,
    /// System-prompt template (the markdown body of WORKFLOW.md).
    /// Supports `{{slot}}` placeholders; v1 substitutes only
    /// `{{date}}` (yesterday's local date) — additional slots land
    /// with the editor UI.
    pub system_prompt: String,
    /// v1512 — notification policy. The WORKFLOW.md frontmatter is
    /// the source of truth (the DB column on `workflow_schedules` is
    /// just a cache for the Ticker). `Default` = `Always`.
    #[serde(default)]
    pub notify_policy: WorkflowNotifyPolicy,
}
