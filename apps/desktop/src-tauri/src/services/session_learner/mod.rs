//! Session memory learning task (memory-system-spec §3.4.2 / §12.3).
//!
//! Fires when a chat thread has been idle long enough that the user
//! has plausibly stopped talking in it. The task reads everything in
//! the thread that hasn't been learned from yet, finds preferences
//! worth promoting to `notes`, and writes a 150–300 字 summary back
//! onto the thread for `thread_search` to use later.
//!
//! Day-1 contract is strict JSON — `prompts/session_memory_learning.md`
//! ends with a schema demand, and `consume_output` parses the agent's
//! final assistant text as `SessionLearnerOutput`. Anything outside
//! the schema gets dropped + a warning logged.

pub mod idle;
pub mod task;

pub use idle::IdleHook;
pub use task::SessionLearnerTask;
