//! Scheduled workflows (v1430).
//!
//! "工作流" = a reusable system-prompt + tool-whitelist definition
//! stored on disk as `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md`.
//! Attaching a [`crate::domain::workflow::Trigger`] to a slug
//! materializes a row in `workflow_schedules`; the [`Ticker`] dispatches
//! the workflow through the shared `BackgroundAgentScheduler` whenever
//! `next_run_at <= now()`.
//!
//! Module layout:
//!
//! | file       | role                                                      |
//! |------------|-----------------------------------------------------------|
//! | `trigger`  | `Trigger::next_after` + validation, + unit tests.         |
//! | `store`    | Filesystem scanner (WORKFLOW.md) + SQLite repo.           |
//! | `task`     | `BackgroundAgentTask` adapter for one scheduled run.      |
//! | `ticker`   | 60s loop that dispatches due schedules.                   |
//!
//! IPC commands live in `crate::commands::workflows` (PR2).

pub mod notify;
pub mod presets;
pub mod store;
pub mod task;
pub mod ticker;
pub mod trigger;

pub use store::{DispatchedSchedule, UpsertSchedule, WorkflowStore};
pub use task::ScheduledWorkflowTask;
pub use ticker::ScheduledWorkflowTicker;
