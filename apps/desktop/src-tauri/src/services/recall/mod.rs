//! Recall data layer.
//!
//! After the chat orchestrator was unified onto Claude CLI, this module
//! shrank to a single survivor:
//!   - `stream`: `Channel<String>` JSON-Lines emitter, reused by
//!     `services::exec_agent` to push events to the chat UI in the same
//!     wire shape the frontend already consumes.
//!
//! The `mcp__corivo__recall_screen_history` MCP tool reaches frames
//! directly via `frames_repo.fts_search` — see
//! `services::exec_agent::mcp_bridge::handle_recall`.

pub mod stream;

pub use stream::{FinishReason, StreamEmitter};
