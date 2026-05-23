# Capture Idle Pause Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a capture-layer `paused_idle` state that pauses screenshots after 4 consecutive frames with `idle_seconds_at_capture >= 15`, resumes on any new input, and surfaces the new state to the frontend.

**Architecture:** Extend `CaptureLoop` with a small internal phase state, idle-frame counter, and a lightweight idle watcher that runs only while paused. Keep the existing pipeline-layer `idle_shortcut` unchanged, and extend the capture-status API plus frontend status UI to render `running / paused_idle / stopped`.

**Tech Stack:** Rust, Tokio, Tauri commands, Vitest, React, TanStack Query

---

### Task 1: Backend Capture Status Contract

**Files:**
- Modify: `src-tauri/src/commands/capture.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/tauri.test.ts`

- [ ] **Step 1: Write the failing frontend wrapper test**

Add an assertion in `src/lib/tauri.test.ts` that `getCaptureStatus()` returns a payload including `phase`, `pause_reason`, and `consecutive_idle_frames`.

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- src/lib/tauri.test.ts`
Expected: FAIL because the mocked `CaptureStatus` shape no longer matches the typed wrapper expectations.

- [ ] **Step 3: Write minimal status-type changes**

Update:
- `src-tauri/src/commands/capture.rs` to serialize the expanded status payload
- `src/lib/types.ts` to add `phase`, `pause_reason`, and `consecutive_idle_frames`

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- src/lib/tauri.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/capture.rs src/lib/types.ts src/lib/tauri.test.ts
git commit -m "feat: extend capture status payload"
```

### Task 2: Backend Pause/Resume State Machine

**Files:**
- Modify: `src-tauri/src/services/capture_loop.rs`

- [ ] **Step 1: Write failing Rust tests for pause and resume**

Add tests covering:
- pause after 4 consecutive idle frames at `>= 15`
- reset when a frame is `< 15`
- no pause when idle data is missing
- resume on any detected input
- manual stop prevents auto-resume

- [ ] **Step 2: Run targeted tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml capture_loop -- --nocapture`
Expected: FAIL with missing phase/state-machine behavior.

- [ ] **Step 3: Write minimal backend implementation**

In `src-tauri/src/services/capture_loop.rs`:
- add `CapturePhase` and `PauseReason`
- track `consecutive_idle_frames`
- add an idle watcher handle
- pause after 4 qualifying frames
- resume when watcher sees new input
- ensure manual stop shuts the watcher down

- [ ] **Step 4: Run targeted tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml capture_loop -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/services/capture_loop.rs
git commit -m "feat: pause capture on consecutive idle frames"
```

### Task 3: Frontend Status Rendering

**Files:**
- Modify: `src/components/layout/status-indicator.tsx`
- Modify: `src/pages/connections/connections-page.tsx`
- Create: `src/components/layout/status-indicator.test.tsx`

- [ ] **Step 1: Write failing status-indicator component tests**

Cover:
- `running` shows `捕获中`
- `paused_idle` shows `无活动暂停`
- `stopped` shows `未启动`

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- src/components/layout/status-indicator.test.tsx`
Expected: FAIL because the component only handles the old boolean status.

- [ ] **Step 3: Write minimal frontend implementation**

Update:
- `StatusIndicator` to render the 3-state text and dot color
- `ConnectionsPage` to reuse the same phase-based status copy for the main card

- [ ] **Step 4: Run tests to verify they pass**

Run: `npm run test -- src/components/layout/status-indicator.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/components/layout/status-indicator.tsx src/components/layout/status-indicator.test.tsx src/pages/connections/connections-page.tsx
git commit -m "feat: show paused idle capture state"
```

### Task 4: Regression Verification

**Files:**
- Modify: `src-tauri/src/services/capture_loop.rs`
- Modify: `src/lib/tauri.test.ts`
- Modify: `src/components/layout/status-indicator.tsx`
- Modify: `src/pages/connections/connections-page.tsx`
- Create: `src/components/layout/status-indicator.test.tsx`

- [ ] **Step 1: Run focused backend verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml capture_loop -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run focused frontend verification**

Run: `npm run test -- src/lib/tauri.test.ts src/components/layout/status-indicator.test.tsx`
Expected: PASS

- [ ] **Step 3: Run broader smoke verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml idle_shortcut -- --nocapture`
Expected: PASS, proving pipeline-layer idle shortcut still works.

- [ ] **Step 4: Record any gaps**

If manual runtime verification was not performed, note that explicitly in the handoff.
