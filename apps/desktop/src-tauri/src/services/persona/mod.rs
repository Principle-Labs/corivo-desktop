//! Persona distillation service (memory-system-spec §4 / §11.4).
//!
//! Two responsibilities:
//!
//! * Run a `PersonaDistillTask` through the background agent task
//!   runner — once a day, plus a manual trigger from Settings.
//! * Post-process the agent's Markdown output: enforce note > persona
//!   precedence by walking each paragraph against the active global
//!   notes, then atomically replace `auto-persona.md`.

pub mod conflict;
pub mod renderer;
pub mod scheduler;
pub mod task;

pub use task::PersonaDistillTask;
