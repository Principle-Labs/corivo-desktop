# Attention Item Notification Judgment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the first runnable version of the Attention Item notification judgment flow from the design spec, including local runtime state, attention resolution, stateful judgment, structured feedback, and prompt debug support.

**Architecture:** Extend the current memory-centric pipeline into `summary -> recall -> attention resolution -> state load -> judgment -> send -> state update`. Persist Attention Item identity, control state, evidence, and events in local SQLite; keep Supermemory as the evidence source; keep JSONL decision logs as audit trails. Use structured LLM outputs for both attention resolution and final interruption judgment, with prompt overrides exposed in settings for debugging.

**Tech Stack:** Rust, SQLite/rusqlite, Tauri commands, React, Vitest, cargo integration tests

---

## File Structure

- `docs/superpowers/specs/2026-04-15-attention-item-notification-judgment-design.md`
  Source-of-truth design for the runtime control plane and LLM contracts.
- `src-tauri/src/domain/config.rs`
  Add attention-resolution prompt debug override semantics.
- `src/lib/types.ts`
  Mirror the new config and overlay feedback request/response shapes.
- `src-tauri/src/db/schema.sql`
  Add runtime tables for attention items, evidence links, and events.
- `src-tauri/src/db/migrations.rs`
  Migrate existing databases to the new schema version.
- `src-tauri/src/db/repos/attention_items.rs`
  New repo for item identity, control state, evidence management, and event writes.
- `src-tauri/src/db/repos/mod.rs`
  Export the new repo.
- `src-tauri/src/db/mod.rs`
  Surface the new repo via `Database`.
- `src-tauri/src/services/notification_decision_log.rs`
  Extend audit logging with attention resolution and judgment details.
- `src-tauri/src/services/mvp_pipeline.rs`
  Main integration point: resolve attention items, load/update state, run structured judgment, and emit notifications.
- `src-tauri/src/commands/notification.rs`
  Upgrade overlay feedback API to structured actions and item-aware control updates.
- `src-tauri/src/lib.rs`
  Wire any new services or state dependencies.
- `src/pages/settings/sections/prompt-debug-section.tsx`
  Add attention-resolution prompt override UI.
- `src/pages/settings/sections/prompt-debug-section.test.tsx`
  Cover the new debug override.
- `src/overlay/overlay-feedback-area.tsx`
  Add quick-action controls on top of optional freeform feedback text.
- `src/overlay/use-overlay-feedback.ts`
  Send structured feedback actions to Tauri.
- `src/lib/tauri.ts`
  Add the structured feedback request shape.
- `src-tauri/tests/config_services.rs`
  Cover new config defaults/normalization.
- `src-tauri/tests/database_layer.rs`
  Cover attention item repo persistence and migrations.
- `src-tauri/tests/mvp_pipeline.rs`
  Cover attention resolution, stateful repeat reminders, suppress/snooze behavior, and decision log output.
- `src/pages/settings/sections/prompt-debug-section.test.tsx`
  Cover the extra prompt override UI.

## Task 1: Add Attention Item Schema, Repo, And Tests

**Files:**
- Create: `src-tauri/src/db/repos/attention_items.rs`
- Modify: `src-tauri/src/db/repos/mod.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/schema.sql`
- Modify: `src-tauri/src/db/migrations.rs`
- Test: `src-tauri/tests/database_layer.rs`

- [ ] **Step 1: Write the failing database tests**

Add tests in `src-tauri/tests/database_layer.rs` for:

- creating an attention item with identity + control fields
- linking multiple evidence memories to the same item
- recording an item event
- updating control state fields like `last_notified_at`, `notify_count`, `snooze_until`, `suppress_until`, `never_notify`

- [ ] **Step 2: Run the focused database tests to verify they fail**

Run: `cargo test --test database_layer attention_item --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because the repo/schema do not exist yet.

- [ ] **Step 3: Add the new schema and migration**

Extend `src-tauri/src/db/schema.sql` and `src-tauri/src/db/migrations.rs` with:

- `attention_items`
- `attention_item_evidence`
- `attention_item_events`
- schema version bump

Keep control fields on `attention_items` for V1 to avoid a fourth table in code while preserving the spec’s semantics.

- [ ] **Step 4: Implement the new repo**

Create `src-tauri/src/db/repos/attention_items.rs` with:

- item create/upsert by `canonical_key`
- evidence replace/list helpers
- control-state update helpers
- event append helper
- list of currently active/open items

- [ ] **Step 5: Export and wire the repo**

Update:

- `src-tauri/src/db/repos/mod.rs`
- `src-tauri/src/db/mod.rs`

- [ ] **Step 6: Run the focused database tests to verify they pass**

Run: `cargo test --test database_layer attention_item --manifest-path src-tauri/Cargo.toml`

Expected: PASS.

## Task 2: Extend Prompt Debug Config For Attention Resolution

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/pages/settings/sections/prompt-debug-section.tsx`
- Test: `src-tauri/tests/config_services.rs`
- Test: `src/pages/settings/sections/prompt-debug-section.test.tsx`

- [ ] **Step 1: Write the failing config and UI tests**

Add tests for:

- config defaulting `attention_resolution_override` to `None`
- blank override normalization to `None`
- prompt debug UI rendering/saving/resetting the third override field

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `cargo test --test config_services attention_resolution --manifest-path src-tauri/Cargo.toml`

Run: `pnpm test -- prompt-debug-section`

Expected: FAIL.

- [ ] **Step 3: Implement the config and UI support**

Add `attention_resolution_override` to the shared config model and extend the settings section with a third textarea plus preview/reset/copy controls.

- [ ] **Step 4: Re-run the focused tests**

Run:

- `cargo test --test config_services attention_resolution --manifest-path src-tauri/Cargo.toml`
- `pnpm test -- prompt-debug-section`

Expected: PASS.

## Task 3: Add Structured Attention Resolution And Judgment To The Pipeline

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/src/services/notification_decision_log.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing pipeline tests**

Add tests covering:

- multiple recalled memories collapse into one attention item
- item state allows re-reminding near deadline even after a previous notification
- `suppress_until` blocks notifications
- `next_check_at` blocks premature re-evaluation
- decision logs include chosen item, `why_now`, `urgency`, `supporting_memory_ids`, `next_check_at`, and `reminder_strategy`

- [ ] **Step 2: Run the focused pipeline tests to verify they fail**

Run: `cargo test --test mvp_pipeline attention_item --manifest-path src-tauri/Cargo.toml`

Expected: FAIL.

- [ ] **Step 3: Implement attention resolution**

In `src-tauri/src/services/mvp_pipeline.rs`:

- add built-in `ATTENTION_RESOLUTION_PROMPT`
- resolve the effective prompt via config override
- call the LLM with summary + recalled memories + open item snapshots
- parse structured item candidates
- derive a deterministic `canonical_key`
- upsert items and evidence in the repo

- [ ] **Step 4: Implement stateful interruption judgment**

In the same pipeline:

- load active item state
- build a structured judgment prompt
- parse outputs including `why_now`, `urgency`, `supporting_memory_ids`, `next_check_at`, `reminder_strategy`
- only hard-block on explicit suppressions and anti-spam guard
- pick at most one foreground notification per batch
- update item state and event history after judge/send

- [ ] **Step 5: Extend audit logging**

Update `src-tauri/src/services/notification_decision_log.rs` to record:

- attention resolution candidates
- chosen item id/title
- recommendation fields
- next-check information

- [ ] **Step 6: Re-run the focused pipeline tests**

Run: `cargo test --test mvp_pipeline attention_item --manifest-path src-tauri/Cargo.toml`

Expected: PASS.

## Task 4: Upgrade Overlay Feedback To Structured Control Actions

**Files:**
- Modify: `src-tauri/src/commands/notification.rs`
- Modify: `src/lib/tauri.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/overlay/overlay-feedback-area.tsx`
- Modify: `src/overlay/use-overlay-feedback.ts`
- Test: `src-tauri/src/commands/notification.rs`
- Test: `src/pages/settings/sections/prompt-debug-section.test.tsx`

- [ ] **Step 1: Write the failing feedback tests**

Add backend tests for:

- `handled` marks the item as done
- `remind_later` writes `snooze_until`
- `stop_for_today` writes `suppress_until`
- `never_remind_this` writes `never_notify`

Add frontend tests for rendering quick-action controls and submitting structured payloads.

- [ ] **Step 2: Run the focused feedback tests to verify they fail**

Run: `cargo test --lib submit_notification_feedback --manifest-path src-tauri/Cargo.toml`

Run: `pnpm test -- overlay-feedback`

Expected: FAIL.

- [ ] **Step 3: Implement the structured feedback flow**

Replace the single freeform action with:

- `handled`
- `remind_later`
- `not_relevant`
- `stop_for_today`
- `never_remind_this`
- optional freeform note

Map each action to item state and event writes while still updating the decision log feedback text for audit.

- [ ] **Step 4: Re-run the focused feedback tests**

Run:

- `cargo test --lib submit_notification_feedback --manifest-path src-tauri/Cargo.toml`
- `pnpm test -- overlay-feedback`

Expected: PASS.

## Task 5: Final Integration Verification

**Files:**
- Modify: `docs/superpowers/specs/2026-04-15-attention-item-notification-judgment-design.md` if implementation deviates materially

- [ ] **Step 1: Run the targeted backend suites**

Run:

- `cargo test --test database_layer attention_item --manifest-path src-tauri/Cargo.toml`
- `cargo test --test mvp_pipeline attention_item --manifest-path src-tauri/Cargo.toml`
- `cargo test --test config_services attention_resolution --manifest-path src-tauri/Cargo.toml`

Expected: PASS.

- [ ] **Step 2: Run the targeted frontend suites**

Run:

- `pnpm test -- prompt-debug-section`
- `pnpm test -- overlay-feedback`

Expected: PASS.

- [ ] **Step 3: Run a broader regression sweep for touched areas**

Run:

- `cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`
- `cargo test --test database_layer --manifest-path src-tauri/Cargo.toml`
- `pnpm test -- prompt-debug-section config-tauri`

Expected: PASS.

- [ ] **Step 4: Commit the implementation**

```bash
git add src-tauri/src/domain/config.rs src/lib/types.ts src-tauri/src/db/schema.sql src-tauri/src/db/migrations.rs src-tauri/src/db/repos/attention_items.rs src-tauri/src/db/repos/mod.rs src-tauri/src/db/mod.rs src-tauri/src/services/notification_decision_log.rs src-tauri/src/services/mvp_pipeline.rs src-tauri/src/commands/notification.rs src/pages/settings/sections/prompt-debug-section.tsx src/overlay/overlay-feedback-area.tsx src/overlay/use-overlay-feedback.ts src/lib/tauri.ts src-tauri/tests/config_services.rs src-tauri/tests/database_layer.rs src-tauri/tests/mvp_pipeline.rs docs/superpowers/plans/2026-04-16-attention-item-notification-judgment.md
git commit -m "feat: add attention item notification judgment flow"
```
