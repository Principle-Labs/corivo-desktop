//! `SnapshotConsumer` trait — the only seam between capture pipeline and
//! storage (spec §四).
//!
//! Capture produces a `SnapshotEnvelope`; a consumer turns that into "data
//! at rest" — for Phase 1 that means writing one row to the local `frames`
//! table. The trait exists so a future `CloudConsumer` (or a dual-write
//! one that does both) can drop in without touching the capture pipeline.
//!
//! The trait surface is **deliberately minimal** — `ingest(env)` only.
//! Anything richer (batching, ack, retry policy) belongs in concrete
//! implementations, not in the trait.

pub mod local;

pub use local::LocalConsumer;

use async_trait::async_trait;

use crate::{domain::snapshot_envelope::SnapshotEnvelope, error::Result};

#[async_trait]
pub trait SnapshotConsumer: Send + Sync {
    /// Persist the envelope. Implementations decide what "persist" means
    /// (local DB write, HTTPS POST, dual-write, …).
    async fn ingest(&self, env: SnapshotEnvelope) -> Result<()>;
}
