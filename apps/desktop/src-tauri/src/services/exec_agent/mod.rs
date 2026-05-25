//! Executive agent — drives the user's natural-language tasks against
//! the bundled `corivo-agent` sidecar.
//!
//! Architecture:
//!   - One Tauri command turn = one `corivo-agent` subprocess
//!     (`runner::run_turn`).
//!   - Multi-turn context is preserved by the sidecar's internal
//!     `SessionManager` (jsonl persistence keyed on `session_id`).
//!   - The subprocess's NDJSON event stream (spec §5.2) is translated
//!     into the same wire shape `services::recall::stream::StreamEmitter`
//!     already emits for `/ask`, so the existing frontend
//!     `useChatStream` hook works unchanged.

pub mod local_context;
pub mod mcp_bridge;
pub mod memory_tools;
mod protocol;
pub mod rpc_server;
pub mod runner;
mod save_note_handler;
mod schedule_task_handler;

pub use local_context::{load as load_local_context, LoadInputs};
pub use mcp_bridge::{McpBridge, PermissionReply};
pub use runner::{run_turn, CorivoAuth, CorivoRunInput};
