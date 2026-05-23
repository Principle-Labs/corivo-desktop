//! `/ask` domain types (spec §五 chat_threads / chat_messages).
//!
//! v1200 redesign: messages carry **content blocks** instead of a flat
//! `content` string + parallel `tool_calls` JSON column. The block
//! shape mirrors Anthropic's Messages API so re-feeding history to the
//! model is verbatim. Lifecycle is split into three commands (`commands::
//! chat::chat_user_message_create` / `chat_assistant_message_start` /
//! `chat_assistant_message_finalize`) so a user's message persists the
//! moment they send — `assistant streaming complete` is no longer a
//! gate on user-message durability.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::domain::config::ApiShape;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatThread {
    pub id: String,
    pub title: Option<String>,
    /// memory-system-spec §11.2. `User` threads appear in the sidebar;
    /// `System` threads are owned by a background agent task and only
    /// surface in the Settings "记忆 → 诊断" panel.
    #[serde(default)]
    pub kind: ChatThreadKind,
    /// memory-system-spec §11.2 — discriminator for `kind=System`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_task: Option<SystemTaskKind>,
    /// Spec §8.2: model the thread was created against. Sidecar reads
    /// `bound_model_id` + `bound_api_shape` instead of the current
    /// Settings selection so a mid-conversation Settings switch doesn't
    /// derail the active turn.
    pub bound_model_id: String,
    pub bound_api_shape: ApiShape,
    /// v1300 sidebar lifecycle. `Some(t)` ↔ thread is pinned (renders
    /// under "置顶", ignores recency sort). `None` ↔ regular thread.
    pub pinned_at: Option<DateTime<Utc>>,
    /// v1300 sidebar lifecycle. `Some(t)` ↔ thread is archived
    /// (hidden from "最近"; surfaced only when "归档(N)" expands).
    /// Archive dominates pin in the UI when both are set.
    pub archived_at: Option<DateTime<Utc>>,
    /// memory-system-spec §12.2 — session learner concise summary.
    /// `None` until the learner has run at least once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_topics: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// memory-system-spec §11.2. Defaults to `User` so legacy callers that
/// only set `bound_model_id` + `title` continue producing user-visible
/// threads.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ChatThreadKind {
    #[default]
    User,
    System,
}

impl ChatThreadKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "user" => Some(Self::User),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// memory-system-spec §10.4 — discriminator for system threads. Open
/// enum (forward-compatible) so a new task lands without breaking
/// existing rows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum SystemTaskKind {
    PersonaDistill,
    SessionMemoryLearning,
}

impl SystemTaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PersonaDistill => "persona_distill",
            Self::SessionMemoryLearning => "session_memory_learning",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "persona_distill" => Some(Self::PersonaDistill),
            "session_memory_learning" => Some(Self::SessionMemoryLearning),
            _ => None,
        }
    }
}

/// Stored role on a `chat_messages` row. v1200 dropped the legacy
/// `tool` variant — tool calls + their results live as blocks inside
/// the assistant turn (see [`ContentBlock::ToolUse`] /
/// [`ContentBlock::ToolResult`]), matching Anthropic's API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    /// memory-system-spec §11.5. Background agent task error records
    /// land as `role='system'` so the diagnostic panel can render them
    /// alongside the actual conversation without leaking into the
    /// user-visible `/ask` history (which is gated to `kind='user'`).
    System,
}

impl ChatRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// One typed content block inside a `chat_messages.content_blocks`
/// JSON array. The serde discriminator + snake_case rename produces
/// the JSON shape `{"type": "text", "text": "..."}`, which lets us
/// round-trip the array straight into Anthropic's `content` payload
/// when we re-feed history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text segment. User input or assistant final answer.
    Text { text: String },

    /// Assistant's reasoning content (Anthropic extended thinking).
    /// `signature` is the per-block signature the model returns —
    /// must be echoed back when re-feeding history with extended
    /// thinking enabled. v1200 captures it as `Option` because the
    /// streaming path doesn't surface it yet; populated by the
    /// finalize step when available.
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },

    /// Assistant invoked a tool. `id` is the tool-use id the matching
    /// `ToolResult` references; `input` is the JSON args object. We
    /// hand-annotate the TS type because `serde_json::Value` doesn't
    /// implement `ts_rs::TS` — `Record<string, unknown>` covers the
    /// 99%-case (tool args are object-shaped) and downstream renderers
    /// already cast to that.
    ToolUse {
        id: String,
        name: String,
        #[ts(type = "Record<string, unknown>")]
        input: serde_json::Value,
    },

    /// Result of a tool call. Sits in the same assistant message
    /// (Anthropic flattens tool turns into a single content array
    /// from the consumer's POV).
    ToolResult {
        tool_use_id: String,
        /// Pre-stringified result. Tool outputs vary wildly in shape;
        /// keeping it `String` (vs. `serde_json::Value`) lets us store
        /// arbitrary text + truncated JSON without schema headaches.
        content: String,
        #[serde(default)]
        is_error: bool,
    },

    /// Quick Ask focus context — what was on screen when the user
    /// hit the global hotkey. Only appears on USER messages. The
    /// conversation log can replay "what the user was looking at"
    /// without consulting the live frames table.
    FocusContext {
        frame_id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        primary_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<String>,
    },

    /// Frames the model surfaced via `recall_screen_history`. Rendered
    /// as 引用卡片 in the UI; mirrored into the row-level
    /// `cited_frame_ids` column for cheap "which threads cite frame X"
    /// lookups.
    FrameCitation { frame_ids: Vec<String> },
}

/// Per-row lifecycle flag. The streaming → complete/error/cancelled
/// transition is the new write path's core invariant — see [`crate::
/// commands::chat`] doc-comments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// Final state. User messages always land here directly; assistant
    /// rows transition Streaming → Complete on a clean finalize.
    Complete,
    /// Assistant placeholder waiting on the agent stream to finish.
    /// Filtered out of `chat_messages_by_thread` reads — only the
    /// boot-time orphan-cleanup pass touches these.
    Streaming,
    /// Stream produced an error before finishing. `error_message` is
    /// non-NULL; `content_blocks` holds whatever blocks landed before
    /// the error.
    Error,
    /// User pressed Stop. `content_blocks` holds the partial output;
    /// no `error_message`.
    Cancelled,
}

impl MessageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Streaming => "streaming",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "complete" => Some(Self::Complete),
            "streaming" => Some(Self::Streaming),
            "error" => Some(Self::Error),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// Token usage captured from the agent's `agent_end` payload. Stored
/// JSON on the assistant row so per-thread / per-day cost queries
/// don't need to re-derive it from blocks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub thread_id: String,
    pub role: ChatRole,
    pub content_blocks: Vec<ContentBlock>,
    /// Plain-text rollup of every `Text` + `Thinking` block, used for
    /// FTS / sidebar preview. May be empty for assistant rows whose
    /// blocks are only tool calls.
    pub content_text: String,
    /// Mirrored from `FrameCitation` blocks (assistant) and the user
    /// message's anchor (Quick Ask). `None` when the row has no
    /// citations.
    pub cited_frame_ids: Option<Vec<String>>,
    pub status: MessageStatus,
    pub error_message: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
    /// Audit: model directory alias that actually drove this turn
    /// (post `resolve_corivo_runtime` fallback). User rows leave it
    /// `None`. Assistant rows from before the v1301 audit landing
    /// will also be `None` — UI should treat that as "unknown model"
    /// rather than a bug.
    pub model_used: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Globally-broadcast event payload for `chat:threads-changed`. Emitted
/// by every Tauri command that mutates `chat_threads` so all webview
/// windows can invalidate their React Query cache off a single source —
/// the main `/ask` page and the Quick Ask overlay are independent JS
/// runtimes with separate `QueryClient` instances, so per-window
/// invalidate alone leaves the other window stale.
///
/// `Updated` covers both rename (auto-title in `chat_user_message_create`)
/// and `updated_at` bumps from new turns; consumers that just refetch the
/// thread list don't need to distinguish.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
pub enum ChatThreadsChanged {
    Created { thread_id: String },
    Deleted { thread_id: String },
    Updated { thread_id: String },
}

impl ChatThreadsChanged {
    pub const EVENT: &'static str = "chat:threads-changed";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_block_serde_round_trips_text() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, r#"{"type":"text","text":"hello"}"#);
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn content_block_serde_round_trips_thinking_with_signature() {
        let block = ContentBlock::Thinking {
            text: "let me think".into(),
            signature: Some("sig_abc".into()),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""signature":"sig_abc""#));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn content_block_serde_skips_null_signature() {
        let block = ContentBlock::Thinking {
            text: "thinking…".into(),
            signature: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(!json.contains("signature"));
    }

    #[test]
    fn content_block_serde_round_trips_tool_use_and_result() {
        let blocks = vec![
            ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "read".into(),
                input: serde_json::json!({"path": "/tmp/x"}),
            },
            ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "file contents".into(),
                is_error: false,
            },
        ];
        let json = serde_json::to_string(&blocks).unwrap();
        let back: Vec<ContentBlock> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, blocks);
    }

    #[test]
    fn message_status_parse_round_trips() {
        for s in ["complete", "streaming", "error", "cancelled"] {
            let parsed = MessageStatus::parse(s).unwrap();
            assert_eq!(parsed.as_str(), s);
        }
        assert!(MessageStatus::parse("nope").is_none());
    }

    #[test]
    fn role_parse_rejects_legacy_tool() {
        // v1200 dropped the 'tool' role — make sure round-trip refuses it.
        assert!(ChatRole::parse("tool").is_none());
        assert!(ChatRole::parse("user").is_some());
        assert!(ChatRole::parse("assistant").is_some());
    }
}
