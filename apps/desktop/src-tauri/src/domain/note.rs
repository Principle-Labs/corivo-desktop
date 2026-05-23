//! Memory `notes` domain types (memory-system-spec §3).
//!
//! Notes are the promoted-declarative layer of the memory system. A
//! `user_explicit` + `global` note feeds the persistent prompt block
//! every turn; other scopes / sources flow through the recall layer.
//!
//! Schema: see `db/schema.sql` for the `notes` table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the note is allowed to influence prompt assembly.
///
/// * `Global` — every thread, unconditional. Drives the persistent
///   `<persistent_memory>` block in `services::exec_agent::local_context`.
/// * `Project` — placeholder; the project model itself hasn't landed.
///   `scope_ref` carries the future `project_id`.
/// * `Session` — bound to a single `chat_threads.id`. Lives in the recall
///   layer only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum NoteScope {
    Global,
    Project,
    Session,
}

impl NoteScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Session => "session",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "global" => Some(Self::Global),
            "project" => Some(Self::Project),
            "session" => Some(Self::Session),
            _ => None,
        }
    }
}

/// Where the note came from. Decides whether it counts as authoritative.
///
/// * `UserExplicit` — user told the agent to remember it. confidence 1.0,
///   `status='active'` by default; flows into the persistent block.
/// * `AgentInferred` — agent (or session learner) thinks it might be a
///   long-term preference. Default confidence 0.6 and `status='suggested'`
///   so the user has to confirm before it influences the persistent block.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum NoteSourceType {
    UserExplicit,
    AgentInferred,
}

impl NoteSourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserExplicit => "user_explicit",
            Self::AgentInferred => "agent_inferred",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "user_explicit" => Some(Self::UserExplicit),
            "agent_inferred" => Some(Self::AgentInferred),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum NoteStatus {
    /// Counts as authoritative; eligible for persistent block / recall.
    Active,
    /// Agent-inferred candidate awaiting user confirmation.
    Suggested,
    /// Replaced by another note (see `superseded_by`).
    Superseded,
    /// Explicitly contradicted by a later authoritative statement.
    Contradicted,
    /// User archived; do not surface unless explicitly asked.
    Archived,
}

impl NoteStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suggested => "suggested",
            Self::Superseded => "superseded",
            Self::Contradicted => "contradicted",
            Self::Archived => "archived",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "suggested" => Some(Self::Suggested),
            "superseded" => Some(Self::Superseded),
            "contradicted" => Some(Self::Contradicted),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct Note {
    pub id: String,
    pub content: String,
    pub scope: NoteScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_ref: Option<String>,
    pub source_type: NoteSourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_id: Option<String>,
    pub confidence: f32,
    pub status: NoteStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_referenced_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}
