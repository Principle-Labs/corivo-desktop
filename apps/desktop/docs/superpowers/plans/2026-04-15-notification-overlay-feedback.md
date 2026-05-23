# Notification Overlay Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Dynamic-Island-style single-line feedback input to the expanded notification overlay and persist submitted feedback back into the matching `notification-decisions.jsonl` entry.

**Architecture:** Extend the overlay notification payload with decision-log identity fields, add a Rust command that updates the existing JSONL decision log by `decision_id`, and add a lightweight frontend feedback sub-state (`idle/submitting/submitted/error`) inside the existing expanded overlay body. Keep backend ownership of overlay phase and keep feedback UI local to the renderer.

**Tech Stack:** Rust, Tauri 2, React, Vitest, tokio JSONL file I/O

---

### Task 1: Define the feedback data contract

**Files:**
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src/lib/types.ts`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write the failing test fixture updates**

Extend the frontend notification fixture so tests assert the overlay payload contains:

```ts
decision_id: "decision-1",
session_id: "session-1",
segment_id: 1,
```

- [ ] **Step 2: Run the focused frontend test and verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: FAIL because the overlay payload types do not include the new fields yet.

- [ ] **Step 3: Add the new Rust and TypeScript fields**

In Rust add nullable identity fields to `OverlayNotificationViewModel`.

In TypeScript mirror them:

```ts
decision_id: string | null;
session_id: string | null;
segment_id: number | null;
```

Also add:

```ts
export interface SubmitNotificationFeedbackRequest {
  decisionId: string;
  text: string;
}
```

- [ ] **Step 4: Re-run the frontend test**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: PASS or later behavior failures, but the type contract compiles.

### Task 2: Add feedback update support to the notification decision logger

**Files:**
- Modify: `src-tauri/src/services/notification_decision_log.rs`
- Test: `src-tauri/src/services/notification_decision_log.rs`

- [ ] **Step 1: Write a failing logger test for updating feedback by `decision_id`**

Add a test that:

- creates a temp session directory
- appends one decision log entry
- calls `update_feedback(decision_id, feedback)`
- re-reads the JSONL file
- asserts the matching line now contains the feedback object

- [ ] **Step 2: Run the logger test and verify it fails**

Run: `cargo test notification_decision_log`

Expected: FAIL because feedback update support does not exist yet.

- [ ] **Step 3: Implement `NotificationDecisionFeedback` and `update_feedback`**

Add:

```rust
pub struct NotificationDecisionFeedback {
    pub text: String,
    pub submitted_at: DateTime<Utc>,
    pub source: String,
}
```

Implement `update_feedback` so it:

- parses each JSONL line
- replaces the matching `decision_id`
- writes the file back atomically
- errors if the decision id is not found

- [ ] **Step 4: Re-run the logger test**

Run: `cargo test notification_decision_log`

Expected: PASS

### Task 3: Expose feedback submission through a Tauri command

**Files:**
- Modify: `src-tauri/src/commands/notification.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/notification_feedback_command.rs`

- [ ] **Step 1: Write a failing command test**

Create `src-tauri/tests/notification_feedback_command.rs` covering:

- valid submit
- blank text
- blank decision id
- unknown decision id

- [ ] **Step 2: Run the command test and verify it fails**

Run: `cargo test --test notification_feedback_command`

Expected: FAIL because the command does not exist.

- [ ] **Step 3: Implement `submit_notification_feedback`**

The command should:

- trim text
- reject empty strings
- reject empty decision id
- call the logger update API

- [ ] **Step 4: Register the command**

Add the command to the Tauri invoke handler in `src-tauri/src/lib.rs`.

- [ ] **Step 5: Re-run the command test**

Run: `cargo test --test notification_feedback_command`

Expected: PASS

### Task 4: Thread decision identity into overlay notifications

**Files:**
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src-tauri/tests/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Add a failing pipeline assertion**

Extend the relevant notification pipeline test so the emitted overlay payload must contain:

- `decision_id`
- `session_id`
- `segment_id`

- [ ] **Step 2: Run the pipeline test and verify it fails**

Run: `cargo test --test mvp_pipeline`

Expected: FAIL because the overlay payload currently lacks those fields.

- [ ] **Step 3: Thread decision identity through the send path**

Update the notification send flow so `overlay.rs` can populate the view model with decision identity.

- [ ] **Step 4: Re-run the pipeline test**

Run: `cargo test --test mvp_pipeline`

Expected: PASS

### Task 5: Add the frontend feedback submission API

**Files:**
- Modify: `src/lib/tauri.ts`
- Modify: `src/lib/types.ts`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write a failing frontend expectation**

Add a test that expects feedback submission to use a typed Tauri invoke wrapper, not a raw event shortcut.

- [ ] **Step 2: Run the frontend test and verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: FAIL because the submit wrapper does not exist yet.

- [ ] **Step 3: Add `submitNotificationFeedback`**

In `src/lib/tauri.ts` add:

```ts
export async function submitNotificationFeedback(
  payload: SubmitNotificationFeedbackRequest,
): Promise<void> {
  return invoke<void>("submit_notification_feedback", payload);
}
```

- [ ] **Step 4: Re-run the frontend test**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: still failing on UI behavior, but the API exists.

### Task 6: Add the Dynamic-Island-style feedback UI

**Files:**
- Modify: `src/overlay/use-overlay-notification.ts`
- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/styles/overlay/island.css`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write failing frontend behavior tests**

Cover:

- expanded notification renders the input and send button
- pressing `Enter` submits
- clicking the send button submits
- successful submit enters `submitted` confirmation state
- failed submit preserves text and shows an error hint

- [ ] **Step 2: Run the focused frontend test and verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: FAIL because the input UI does not exist yet.

- [ ] **Step 3: Add feedback local state to `use-overlay-notification.ts`**

Track:

- current input text
- `idle/submitting/submitted/error`
- submit handler
- success confirmation timeout

Keep backend ownership of overlay phase unchanged.

- [ ] **Step 4: Render the input tray in `overlay-app.tsx`**

In `expanded` state render:

- a single-line input
- a send button
- success/error microcopy

Wire `Enter` and button click to the submit handler.

- [ ] **Step 5: Style the tray in `island.css`**

Add:

- low-contrast inset input styling
- minimal send affordance
- subtle focus ring
- restrained success/error micro states

- [ ] **Step 6: Re-run the frontend test**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: PASS

### Task 7: Verify the focused surface

**Files:**
- Verify: `src-tauri/src/services/notification_decision_log.rs`
- Verify: `src-tauri/src/commands/notification.rs`
- Verify: `src-tauri/src/providers/notification/overlay.rs`
- Verify: `src/lib/tauri.ts`
- Verify: `src/overlay/use-overlay-notification.ts`
- Verify: `src/overlay/overlay-app.tsx`
- Verify: `src/styles/overlay/island.css`
- Verify: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Run logger tests**

Run: `cargo test notification_decision_log`

Expected: PASS

- [ ] **Step 2: Run command tests**

Run: `cargo test --test notification_feedback_command`

Expected: PASS

- [ ] **Step 3: Run pipeline identity tests**

Run: `cargo test --test mvp_pipeline`

Expected: PASS for the focused notification identity assertions.

- [ ] **Step 4: Run the focused frontend overlay tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: PASS

- [ ] **Step 5: Inspect the final diff**

Run: `git diff -- src-tauri/src/providers/notification/types.rs src-tauri/src/providers/notification/overlay.rs src-tauri/src/services/notification_decision_log.rs src-tauri/src/commands/notification.rs src/lib/types.ts src/lib/tauri.ts src/overlay/use-overlay-notification.ts src/overlay/overlay-app.tsx src/styles/overlay/island.css docs/superpowers/specs/2026-04-15-notification-overlay-feedback-design.md docs/superpowers/plans/2026-04-15-notification-overlay-feedback.md`

Expected: only overlay feedback UI, feedback persistence, and related tests/docs changed
