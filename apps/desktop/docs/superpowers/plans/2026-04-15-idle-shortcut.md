# Idle Shortcut Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a conservative idle shortcut to the screenshot pipeline so high-confidence idle batches create only an internal idle segment and skip Gemini, memory update/search, judgment, and notification work.

**Architecture:** Keep runtime screenshot persistence on the existing filesystem `CaptureStore` path. Extend capture batches with per-frame idle metadata, add a pure Rust idle classifier service, and integrate it at the top of `MvpPipeline::process_batch()`. Persist shortcut evidence on the segment row via `shortcut_metadata`, and make session-document aggregation ignore idle segments completely.

**Tech Stack:** Rust, Tauri, Tokio, rusqlite/SQLite migrations, Serde JSON, existing `CaptureLoop`/`MvpPipeline`/repo tests

---

## File Structure

- Modify: `src-tauri/src/db/schema.sql`
  Add `segments.shortcut_metadata` and keep `activity_type` as the segment-type switch.
- Modify: `src-tauri/src/db/migrations.rs`
  Add a new migration for the `segments.shortcut_metadata` column and bump schema version.
- Modify: `src-tauri/src/db/repos/segments.rs`
  Extend `Segment`, `SegmentInput`, `SegmentUpdate`, row mapping, insert, and update support for `activity_type` and `shortcut_metadata`.
- Modify: `src-tauri/src/services/capture_loop.rs`
  Introduce `CapturedFrame`, collect `captured_at` plus `idle_seconds_at_capture`, and make `CapturedBatch` frame-aware while preserving convenience accessors for existing callers/tests.
- Create: `src-tauri/src/services/idle_shortcut.rs`
  Implement the pure idle-classification logic and typed `IdleShortcutMetadata`.
- Modify: `src-tauri/src/services/mod.rs`
  Export the new idle shortcut module.
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
  Add the idle gate, internal idle segment creation, logging, and aggregation filtering.
- Modify: `src-tauri/src/commands/memory.rs`
  Ensure `backfill_session_documents_impl()` ignores idle segments and computes session metadata from non-idle segments only.
- Modify: `src-tauri/tests/database_layer.rs`
  Cover the new segment column/repo behavior.
- Modify: `src-tauri/tests/mvp_pipeline.rs`
  Cover idle shortcut hit/miss behavior and assert no heavy pipeline calls on idle batches.
- Modify: `src-tauri/tests/memory_commands.rs`
  Cover backfill/session-document exclusion of idle segments.

### Task 1: Extend Segment Persistence For Idle Metadata

**Files:**
- Modify: `src-tauri/src/db/schema.sql`
- Modify: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/repos/segments.rs`
- Test: `src-tauri/tests/database_layer.rs`

- [ ] **Step 1: Write the failing database-layer test**

Add a test in `src-tauri/tests/database_layer.rs` that:
- creates a segment with `activity_type = "idle_shortcut"`
- sets `shortcut_metadata` to a JSON string
- reads it back through `SegmentsRepo::get()`
- updates the same segment and verifies the new metadata persists

- [ ] **Step 2: Run the targeted test to verify it fails**

Run: `cargo test --test database_layer`
Expected: FAIL because `segments.shortcut_metadata` is missing and/or repo structs cannot read/write `activity_type` and `shortcut_metadata`.

- [ ] **Step 3: Add the schema migration**

Update `src-tauri/src/db/schema.sql` and `src-tauri/src/db/migrations.rs` so the DB includes:

```sql
ALTER TABLE segments ADD COLUMN shortcut_metadata TEXT;
```

Also bump `LATEST_VERSION` and add a guarded migration function similar to `apply_v2`.

- [ ] **Step 4: Extend the segment repo model**

Update `src-tauri/src/db/repos/segments.rs` so:
- `Segment` includes `shortcut_metadata: Option<String>`
- `SegmentInput` includes optional `activity_type` and `shortcut_metadata`
- `SegmentUpdate` includes optional `activity_type` and `shortcut_metadata`
- insert/update SQL writes them
- `row_to_segment()` reads them

- [ ] **Step 5: Re-run the database-layer test**

Run: `cargo test --test database_layer`
Expected: PASS for the new segment metadata coverage.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/schema.sql src-tauri/src/db/migrations.rs src-tauri/src/db/repos/segments.rs src-tauri/tests/database_layer.rs
git commit -m "feat: persist idle shortcut metadata on segments"
```

### Task 2: Make Capture Batches Frame-Aware

**Files:**
- Modify: `src-tauri/src/services/capture_loop.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Add a failing pipeline test fixture update**

Update one `CapturedBatch` construction in `src-tauri/tests/mvp_pipeline.rs` to use a frame-aware batch shape:

```rust
CapturedBatch {
    session_id: "session-1".to_string(),
    frames: vec![/* frame fixtures */],
}
```

Expected failure: compile errors because `CapturedBatch` still exposes only `images` and `paths`.

- [ ] **Step 2: Run the targeted pipeline test to verify it fails**

Run: `cargo test --test mvp_pipeline`
Expected: FAIL to compile due to the new batch shape.

- [ ] **Step 3: Introduce `CapturedFrame` and batch helpers**

In `src-tauri/src/services/capture_loop.rs`:
- add `CapturedFrame { image, path, captured_at, idle_seconds_at_capture }`
- change `CapturedBatch` to `frames: Vec<CapturedFrame>`
- add small helper methods if useful:

```rust
impl CapturedBatch {
    pub fn image_count(&self) -> usize { self.frames.len() }
    pub fn images(&self) -> Vec<ImageInput> { ... }
    pub fn paths(&self) -> Vec<PathBuf> { ... }
}
```

- [ ] **Step 4: Populate frame metadata at capture time**

In `CaptureLoop::start()`:
- capture `Utc::now()` for each screenshot
- read the current HID idle seconds via a small helper
- store both on each `CapturedFrame`

Keep filesystem persistence unchanged. Do not write SQLite `screenshots` rows in this task.

- [ ] **Step 5: Re-run the pipeline test suite**

Run: `cargo test --test mvp_pipeline`
Expected: PASS compile for the new batch shape, though idle behavior is not implemented yet.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/services/capture_loop.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: attach idle metadata to captured frames"
```

### Task 3: Add A Pure Idle Classifier Service

**Files:**
- Create: `src-tauri/src/services/idle_shortcut.rs`
- Modify: `src-tauri/src/services/mod.rs`

- [ ] **Step 1: Write the failing classifier tests**

Create `src-tauri/src/services/idle_shortcut.rs` with unit tests for:
- shortcut hit when coverage and qualified ratio are high
- miss when any frame has `idle_seconds_at_capture < 45`
- miss when frame count is `< 4`
- miss when coverage ratio is below `0.85`
- miss when any frame lacks idle data

- [ ] **Step 2: Run the new unit tests to verify they fail**

Run: `cargo test idle_shortcut`
Expected: FAIL because the module and classifier logic do not exist yet.

- [ ] **Step 3: Implement the classifier and metadata**

Implement:
- pure input structs that do not depend on Tauri/runtime
- `assess_idle_batch(...) -> Option<IdleShortcutMetadata>`
- `IdleShortcutMetadata` including:
  - `shortcut_type`
  - `paths`
  - `screenshot_count`
  - `qualified_idle_ratio`
  - `coverage_ratio`
  - `min/max/last_idle_seconds_at_capture`
  - threshold values

Use the agreed rules:
- all frames must carry idle seconds
- at least 4 frames
- at least 90% frames with idle >= 120
- last frame idle >= 120
- any frame idle < 45 is an automatic miss
- coverage ratio >= 0.85

- [ ] **Step 4: Export the module**

Update `src-tauri/src/services/mod.rs` to expose `idle_shortcut`.

- [ ] **Step 5: Re-run the unit tests**

Run: `cargo test idle_shortcut`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/services/idle_shortcut.rs src-tauri/src/services/mod.rs
git commit -m "feat: add conservative idle shortcut classifier"
```

### Task 4: Integrate Idle Shortcut Into The Pipeline

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Add failing pipeline tests for the shortcut hit path**

In `src-tauri/tests/mvp_pipeline.rs`, add a test that:
- builds an idle batch with frame timestamps and idle seconds
- runs `process_batch()`
- asserts:
  - no fake LLM calls
  - no fake memory add/update/search
  - no notifier calls
  - one segment exists with `activity_type = "idle_shortcut"`
  - `shortcut_metadata` includes all batch paths

- [ ] **Step 2: Add a failing pipeline test for the miss path**

Add a second test where one frame has `idle_seconds_at_capture < 45`, and assert the normal LLM path still runs.

- [ ] **Step 3: Run the pipeline test suite to verify failure**

Run: `cargo test --test mvp_pipeline`
Expected: FAIL because the idle gate does not exist yet.

- [ ] **Step 4: Add the idle gate at the top of `process_batch()`**

In `src-tauri/src/services/mvp_pipeline.rs`:
- convert the batch into classifier input
- call the new idle classifier before any LLM or memory work
- if matched:
  - create a segment using the real first/last `captured_at`
  - set `status = done`
  - set `activity_type = "idle_shortcut"`
  - set `summary = "用户在这段时间内处于空闲状态。"`
  - set `cost_usd = 0.0`
  - set `shortcut_metadata` to serialized JSON
  - return early

Do not emit memory events. Do not write notification decision logs for idle batches.

- [ ] **Step 5: Add tracing for observability**

Emit a concise `tracing::info!` when the shortcut hits, including session id, frame count, coverage ratio, and qualified ratio.

- [ ] **Step 6: Re-run the pipeline tests**

Run: `cargo test --test mvp_pipeline`
Expected: PASS for both shortcut-hit and shortcut-miss paths.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/services/mvp_pipeline.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: skip heavy pipeline work for idle batches"
```

### Task 5: Exclude Idle Segments From Session Document Aggregation

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/src/commands/memory.rs`
- Test: `src-tauri/tests/memory_commands.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Add the failing aggregation tests**

Add tests that prove:
- `build_session_document()` ignores `activity_type = "idle_shortcut"`
- `backfill_session_documents_impl()` ignores idle segments
- session-document metadata uses only non-idle segments for:
  - `segment_count`
  - `last_segment_id`
  - `occurred_at`

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test --test memory_commands`
Expected: FAIL because idle segments are still included by the current `summary.is_some()` logic.

- [ ] **Step 3: Implement the non-idle filter**

Update:
- `MvpPipeline::build_session_document()`
- the normal `process_batch()` aggregation path
- `backfill_session_documents_impl()`

to derive document content and metadata only from non-idle summarized segments.

If a session contains only idle segments, the code should skip memory document creation/update entirely.

- [ ] **Step 4: Re-run the aggregation tests**

Run: `cargo test --test memory_commands`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/mvp_pipeline.rs src-tauri/src/commands/memory.rs src-tauri/tests/memory_commands.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "fix: exclude idle shortcuts from session document sync"
```

### Task 6: Full Verification

**Files:**
- Modify: `docs/superpowers/specs/2026-04-14-idle-shortcut-design.md`
  Only if implementation diverges from the approved behavior.

- [ ] **Step 1: Run the focused backend suites**

Run: `cargo test --test database_layer --test mvp_pipeline --test memory_commands`
Expected: PASS.

- [ ] **Step 2: Run the full backend test suite**

Run: `cargo test`
Expected: PASS without idle-shortcut regressions.

- [ ] **Step 3: Manually inspect the main changed surfaces**

Check:
- the migration version bump is correct
- `CapturedBatch` helper methods keep existing callers simple
- no idle batch writes to memory/search/notifier paths
- `shortcut_metadata.paths` stores the full batch path list

- [ ] **Step 4: Commit any final doc alignment if needed**

```bash
git add docs/superpowers/specs/2026-04-14-idle-shortcut-design.md
git commit -m "docs: align idle shortcut spec with implementation details"
```

Only do this if the code forced a spec clarification.
