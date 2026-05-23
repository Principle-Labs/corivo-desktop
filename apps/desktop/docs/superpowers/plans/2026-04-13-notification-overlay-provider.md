# Notification Overlay Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current hard-coded system notification path with a provider-backed overlay notification implementation.

**Architecture:** Add a notification provider/service layer in Rust, wire `MvpPipeline` and the test command to that service, then render notifications in a dedicated transparent Tauri overlay window with a separate frontend entry.

**Tech Stack:** Rust, Tauri 2, React 19, Vite 7, Vitest

---

### Task 1: Rust notification config and service

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src-tauri/src/domain/mod.rs`
- Modify: `src-tauri/src/providers/mod.rs`
- Create: `src-tauri/src/providers/notification/mod.rs`
- Create: `src-tauri/src/providers/notification/types.rs`
- Create: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Create: `src-tauri/src/services/notification_service.rs`
- Test: `src-tauri/tests/config_services.rs`
- Test: `src-tauri/tests/notification_service.rs`

- [ ] Write failing tests for config defaults and provider selection
- [ ] Run Rust tests to confirm failure
- [ ] Implement notification config, provider types, and notification service
- [ ] Re-run focused Rust tests until green

### Task 2: Tauri wiring and pipeline integration

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src-tauri/src/commands/notification.rs`
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] Write failing tests for pipeline notification payload delivery
- [ ] Run focused Rust tests to confirm failure
- [ ] Wire `NotificationService` into app state, test command, and pipeline
- [ ] Re-run focused Rust tests until green

### Task 3: Overlay renderer

**Files:**
- Modify: `vite.config.ts`
- Create: `notification-overlay.html`
- Create: `src/overlay/main.tsx`
- Create: `src/overlay/overlay-app.tsx`
- Create: `src/overlay/overlay-app.test.tsx`
- Modify: `src/styles/globals.css`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/tauri.test.ts`

- [ ] Write failing frontend test for overlay state transitions
- [ ] Run focused Vitest to confirm failure
- [ ] Implement overlay entry, event listener, animation state machine, and styles
- [ ] Re-run focused frontend tests until green

### Task 4: Full verification

**Files:**
- Test: `src-tauri/tests/notification_service.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`
- Test: `src/lib/tauri.test.ts`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] Run focused Rust and frontend tests
- [ ] Run broader `cargo test` and `pnpm test` verification as time permits
- [ ] Summarize what changed, what passed, and any remaining runtime-only risks
