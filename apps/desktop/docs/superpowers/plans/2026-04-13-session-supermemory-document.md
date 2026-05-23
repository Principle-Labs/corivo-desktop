# Session-Scoped Supermemory Document Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make each Corivo capture session map to exactly one Supermemory document, with later batches updating that same document instead of creating new ones.

**Architecture:** Persist each batch as a local segment, derive one session-level aggregate document from all segments in that session, and sync that aggregate to Supermemory via `add` on first write and `update` on subsequent writes. Store the canonical remote document identity on the local `sessions` row so retries and future updates are deterministic.

**Tech Stack:** Rust, Tauri, rusqlite, reqwest, tokio, SQLite, Supermemory v3 API

---

### Task 1: Add Session-Level Supermemory Identity to SQLite

**Files:**
- Modify: `src-tauri/src/db/schema.sql`
- Modify: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/repos/sessions.rs`
- Test: `src-tauri/tests/database_layer.rs`

- [ ] **Step 1: Write the failing database test**

Add a test in `src-tauri/tests/database_layer.rs` that:

- creates a session
- saves `supermemory_document_id = "doc-1"`
- saves `supermemory_custom_id = "corivo:session:session-1"`
- reloads the session
- asserts both fields round-trip correctly

- [ ] **Step 2: Run the new test to verify it fails**

Run: `cargo test --test database_layer sessions_repo_persists_supermemory_identity --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because the `sessions` table and repo do not expose those fields yet.

- [ ] **Step 3: Add schema v2**

Update `src-tauri/src/db/schema.sql` and `src-tauri/src/db/migrations.rs`:

- bump `LATEST_VERSION` from `1` to `2`
- add `ALTER TABLE sessions ADD COLUMN supermemory_document_id TEXT`
- add `ALTER TABLE sessions ADD COLUMN supermemory_custom_id TEXT`
- insert schema version `2`
- keep the migration idempotent for existing local databases

- [ ] **Step 4: Extend the session model and repo**

Update `src-tauri/src/db/repos/sessions.rs`:

- add the two fields to `Session`
- load them in `row_to_session`
- add a repo method similar to:

```rust
pub async fn set_supermemory_identity(
    &self,
    session_id: String,
    document_id: String,
    custom_id: String,
) -> Result<()>
```

- [ ] **Step 5: Re-run the database test**

Run: `cargo test --test database_layer sessions_repo_persists_supermemory_identity --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/schema.sql src-tauri/src/db/migrations.rs src-tauri/src/db/repos/sessions.rs src-tauri/tests/database_layer.rs
git commit -m "feat: store supermemory identity on sessions"
```

### Task 2: Expose Session Segment Listing for Aggregate Rebuilds

**Files:**
- Modify: `src-tauri/src/db/repos/segments.rs`
- Test: `src-tauri/tests/database_layer.rs`

- [ ] **Step 1: Write the failing repo test**

Add a test that creates two segments for the same `session_id` and verifies `list_by_session("session-1")` returns them ordered by `started_at ASC`.

- [ ] **Step 2: Run the new test to verify it fails**

Run: `cargo test --test database_layer segments_repo_lists_by_session --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because `list_by_session` does not exist.

- [ ] **Step 3: Implement the repo method**

Add this method to `src-tauri/src/db/repos/segments.rs`:

```rust
pub async fn list_by_session(&self, session_id: String) -> Result<Vec<Segment>>
```

Use:

- `WHERE session_id = ?1`
- `ORDER BY started_at ASC`

- [ ] **Step 4: Re-run the repo test**

Run: `cargo test --test database_layer segments_repo_lists_by_session --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/repos/segments.rs src-tauri/tests/database_layer.rs
git commit -m "feat: list segments by session"
```

### Task 3: Extend Memory Contracts for First-Write Identity and Later Updates

**Files:**
- Modify: `src-tauri/src/providers/memory/types.rs`
- Modify: `src-tauri/src/providers/memory/mod.rs`
- Modify: `src-tauri/src/providers/memory/supermemory.rs`
- Modify: `src-tauri/src/services/memory_service.rs`
- Test: `src-tauri/tests/memory_flow.rs`
- Test: `src-tauri/src/providers/memory/supermemory.rs`

- [ ] **Step 1: Write the failing provider tests**

Add tests covering:

- `MemoryInput.external_id` is serialized to the Supermemory add payload as `customId`
- `SupermemoryProvider::update()` sends the correct request shape
- `MemoryService` exposes the new update path

- [ ] **Step 2: Run the provider-focused tests**

Run: `cargo test supermemory --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because `external_id` and `update()` do not exist.

- [ ] **Step 3: Extend `MemoryInput` and `MemoryProvider`**

Update `src-tauri/src/providers/memory/types.rs`:

```rust
pub struct MemoryInput {
    pub content: String,
    pub source: MemorySource,
    pub metadata: serde_json::Value,
    pub tags: Vec<String>,
    pub occurred_at: DateTime<Utc>,
    pub external_id: Option<String>,
}
```

Update `src-tauri/src/providers/memory/mod.rs`:

```rust
async fn update(&self, id: &MemoryId, memory: MemoryInput) -> Result<(), types::MemoryError>;
```

- [ ] **Step 4: Implement the Supermemory mapping**

Update `src-tauri/src/providers/memory/supermemory.rs`:

- include `customId` in add payload when `external_id` exists
- implement `PATCH /v3/documents/{id}`
- reuse the same metadata flattening logic for both add and update payloads

- [ ] **Step 5: Wire the service layer**

Add an `update()` method to `src-tauri/src/services/memory_service.rs` that forwards to the provider.

- [ ] **Step 6: Re-run the provider tests**

Run: `cargo test supermemory --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/providers/memory/types.rs src-tauri/src/providers/memory/mod.rs src-tauri/src/providers/memory/supermemory.rs src-tauri/src/services/memory_service.rs src-tauri/tests/memory_flow.rs
git commit -m "feat: support session-scoped memory updates"
```

### Task 4: Persist Batches as Segments and Build Session Aggregate Documents

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing pipeline test**

Replace the existing “one batch = one add” assumption with tests that verify:

- first batch for a session calls `add`
- second batch for the same session calls `update`
- both batches point to the same canonical memory id
- emitted memory event uses the canonical session document id

- [ ] **Step 2: Run the pipeline tests**

Run: `cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because the pipeline currently always calls `add`.

- [ ] **Step 3: Inject database-backed batch persistence**

Refactor `src-tauri/src/services/mvp_pipeline.rs` to:

- create a segment row per batch
- assign screenshots in the batch to that segment when screenshot row ids are available
- save summary / tokens / cost / generated_at to the segment
- load all session segments
- build one aggregate document string

Recommended helper shape inside the file:

```rust
fn build_session_document(session_id: &str, segments: &[Segment]) -> String
```

- [ ] **Step 4: Add identity-aware sync logic**

Still in `src-tauri/src/services/mvp_pipeline.rs`:

- compute `custom_id = format!("corivo:session:{session_id}")`
- if session has no `supermemory_document_id`, call `memory.add(...)`
- if it does, call `memory.update(...)`
- on first success, persist `document_id/custom_id` back to `sessions`
- always write the canonical document id into `segment.memory_id`

- [ ] **Step 5: Re-run the pipeline tests**

Run: `cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/services/mvp_pipeline.rs src-tauri/src/commands/config.rs src-tauri/src/lib.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: sync one supermemory document per session"
```

### Task 5: Add a Deterministic Backfill Command for Existing Sessions

**Files:**
- Modify: `src-tauri/src/commands/memory.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/memory_commands.rs`

- [ ] **Step 1: Write the failing command test**

Add a command-level test that:

- seeds a session plus two summarized segments
- runs a new backfill command
- asserts the session gains a canonical `supermemory_document_id`

- [ ] **Step 2: Run the command test**

Run: `cargo test --test memory_commands --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because the backfill command does not exist.

- [ ] **Step 3: Implement the backfill command**

Add a command like:

```rust
#[tauri::command]
pub async fn backfill_session_documents(...) -> Result<i64, String>
```

Behavior:

- iterate sessions
- skip sessions with no summarized segments
- build aggregate content from local segments
- create or update the canonical document
- return the count of processed sessions

- [ ] **Step 4: Re-run the command test**

Run: `cargo test --test memory_commands --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/memory.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/memory_commands.rs
git commit -m "feat: backfill canonical session documents"
```

### Task 6: Verify End-to-End Behavior Without Depending on Search

**Files:**
- Modify: `src-tauri/tests/memory_flow.rs`
- Modify: `src-tauri/tests/mvp_pipeline.rs`
- Optional doc note: `docs/superpowers/specs/2026-04-13-session-supermemory-document-design.md`

- [ ] **Step 1: Add regression coverage**

Add tests that assert:

- retries reuse `external_id` instead of creating a new canonical mapping
- `sessions.supermemory_document_id` remains stable across multiple batches
- a failed update leaves enough local state to retry safely

- [ ] **Step 2: Run the focused regression suite**

Run:

```bash
cargo test --test memory_flow --manifest-path src-tauri/Cargo.toml
cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml
cargo test --test database_layer --manifest-path src-tauri/Cargo.toml
```

Expected: PASS

- [ ] **Step 3: Run the broader backend suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: PASS, or only unrelated existing failures.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/memory_flow.rs src-tauri/tests/mvp_pipeline.rs src-tauri/tests/database_layer.rs
git commit -m "test: cover canonical session memory sync"
```

### Task 7: Manual QA Against a Real Supermemory Account

**Files:**
- No code changes required
- Reference: `src-tauri/src/providers/memory/supermemory.rs`
- Reference: `docs/superpowers/specs/2026-04-13-session-supermemory-document-design.md`

- [ ] **Step 1: Start with a clean local session**

Run the app with a valid Supermemory key configured.

- [ ] **Step 2: Create one session with multiple batches**

Trigger at least two pipeline batches within the same session.

Expected:

- only one canonical document is associated with the session locally
- later batches update that same document

- [ ] **Step 3: Validate with direct document fetch, not search**

Use the saved `supermemory_document_id` and confirm:

- document content includes all batch summaries for the session
- no second canonical document is created for that session

- [ ] **Step 4: Run a backfill on pre-existing sessions**

Expected:

- sessions with summaries gain a canonical document mapping
- sessions without summaries are skipped cleanly

- [ ] **Step 5: Record follow-ups**

If manual QA still shows duplicates, inspect:

- local `sessions.supermemory_document_id`
- add payload `customId`
- update request target id

