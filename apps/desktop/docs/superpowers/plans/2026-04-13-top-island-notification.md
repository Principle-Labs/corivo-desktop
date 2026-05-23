# Top Island Notification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the bottom overlay notification with a top-centered island that expands on hover and collapses after a short delay.

**Architecture:** Keep the existing notification provider/service stack, but change the overlay window geometry to a top-centered compact island and move the interaction model to a compact/expanded island state machine in the renderer. The same window stays alive while frontend hover events and window API calls control expansion and collapse.

**Tech Stack:** Rust, Tauri 2, React 19, Vitest

---

### Task 1: Lock the new interaction model with tests

**Files:**
- Modify: `src/overlay/overlay-app.test.tsx`
- Modify: `src-tauri/tests/config_services.rs`

- [ ] Write failing tests for compact render, hover expand, delayed collapse, and new overlay config defaults
- [ ] Run focused frontend and Rust tests to verify they fail for the expected reasons
- [ ] Keep the assertions minimal and behavior-focused
- [ ] Re-run the same tests after each implementation slice

### Task 2: Update notification config and overlay view model

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/config-tauri.test.ts`

- [ ] Add compact width/height, expanded width/height, and collapse delay config
- [ ] Extend the overlay view model to carry both compact and expanded geometry
- [ ] Update TypeScript config types and wrapper tests
- [ ] Run focused tests to keep config changes green

### Task 3: Rebuild top-island renderer and window geometry

**Files:**
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/styles/globals.css`

- [ ] Write minimal code to move the initial window to top center and stop ignoring cursor events
- [ ] Implement compact/expanded/delay state handling in the renderer
- [ ] Resize and reposition the current window from the renderer as state changes
- [ ] Replace the bottom-band visuals with the top-island black capsule UI

### Task 4: Verify the full notification stack

**Files:**
- Test: `src/overlay/overlay-app.test.tsx`
- Test: `src-tauri/tests/config_services.rs`
- Test: `src-tauri/tests/notification_service.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] Run focused tests for the island renderer and config
- [ ] Run full `pnpm test`
- [ ] Run full `cargo test --manifest-path src-tauri/Cargo.toml`
- [ ] Run `pnpm build` and confirm the overlay entry still builds
