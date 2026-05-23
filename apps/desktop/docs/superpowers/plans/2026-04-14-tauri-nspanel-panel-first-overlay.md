# Tauri NSPanel Panel-First Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild Corivo's macOS notification overlay around `tauri-nspanel` so the overlay is a real panel with backend-owned lifecycle and frame switching, while keeping the current React island UI and notch-aware geometry model.

**Architecture:** Keep `screen_metrics.rs` and the React overlay content layer, but insert a new `panel.rs` controller between the notification provider and the native container. The backend will pre-create and reuse a `tauri-nspanel` panel, cache the current overlay metrics, and expose a small Tauri command for compact/expanded/hidden boundary updates so the frontend stops mutating the native window directly.

**Tech Stack:** Rust, Tauri 2, `tauri-nspanel`, React 19, Vitest, Cargo tests, existing notch-aware overlay geometry

---

## File Structure

- Modify: `src-tauri/Cargo.toml`
  Responsibility: add `tauri-nspanel` and trim macOS-specific manual panel dependencies where possible.
- Modify: `src-tauri/Cargo.lock`
  Responsibility: lock the new dependency graph after adding `tauri-nspanel`.
- Create: `src-tauri/src/providers/notification/panel.rs`
  Responsibility: own native panel creation, reuse, show/hide, and frame switching.
- Modify: `src-tauri/src/providers/notification/mod.rs`
  Responsibility: export the new panel module.
- Modify: `src-tauri/src/providers/notification/overlay.rs`
  Responsibility: remain the notification provider, but delegate all native panel actions to `panel.rs`.
- Modify: `src-tauri/src/providers/notification/types.rs`
  Responsibility: add any panel-facing state enums or cached overlay state types needed by the controller.
- Modify: `src-tauri/src/lib.rs`
  Responsibility: pre-create the panel during app setup instead of patching a generic window.
- Modify: `src-tauri/src/commands/notification.rs`
  Responsibility: add a command for frontend phase-to-panel boundary sync.
- Modify: `src-tauri/src/commands/mod.rs`
  Responsibility: export any new notification command types if needed.
- Create: `src-tauri/tests/notification_panel_controller.rs`
  Responsibility: lock panel controller lifecycle and frame-switch behavior with a fake adapter.
- Modify: `src-tauri/tests/notification_overlay_geometry.rs`
  Responsibility: keep geometry assertions aligned with the refactor.
- Modify: `src/lib/tauri.ts`
  Responsibility: expose the new overlay panel sync command for the frontend.
- Modify: `src/lib/tauri.test.ts`
  Responsibility: lock the invoke name and payload for the new panel sync command.
- Modify: `src/overlay/use-overlay-notification.ts`
  Responsibility: replace direct `getCurrentWindow().setSize/setPosition/show/hide` calls with backend panel sync invokes.
- Modify: `src/overlay/overlay-app.test.tsx`
  Responsibility: verify the frontend now signals backend phase changes rather than mutating the native window directly.

## Task 1: Lock Panel-First Lifecycle with Failing Rust Tests

**Files:**
- Create: `src-tauri/tests/notification_panel_controller.rs`
- Test: `src-tauri/src/providers/notification/panel.rs`

- [ ] **Step 1: Write failing tests for panel controller reuse and frame switching**

Add tests that describe a fake panel adapter contract:

- `ensure_panel` creates the panel only once
- `show_panel` reuses the existing panel
- `update_panel_frame` switches between compact and expanded bounds without recreating
- `hide_panel` hides but does not destroy the panel

Use a fake adapter that records `create`, `show`, `hide`, and `set_frame` calls.

- [ ] **Step 2: Run the focused Rust test to verify failure**

Run: `cargo test --test notification_panel_controller`
Workdir: `src-tauri`
Expected: FAIL because `panel.rs` and its controller types do not exist yet.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/notification_panel_controller.rs
git commit -m "test: define panel-first overlay controller contract"
```

## Task 2: Add `tauri-nspanel` and Implement the Native Panel Controller

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Create: `src-tauri/src/providers/notification/panel.rs`
- Modify: `src-tauri/src/providers/notification/mod.rs`
- Modify: `src-tauri/src/providers/notification/types.rs`
- Test: `src-tauri/tests/notification_panel_controller.rs`

- [ ] **Step 1: Add the dependency**

Add `tauri-nspanel` in `src-tauri/Cargo.toml` using the source/version recommended by the project README, then refresh `Cargo.lock`.

- [ ] **Step 2: Implement a testable panel controller abstraction**

Create:

```rust
pub enum OverlayPanelPhase {
    Compact,
    Expanded,
    Hidden,
}

pub struct OverlayPanelState {
    pub compact_frame: ...
    pub expanded_frame: ...
    pub current_phase: OverlayPanelPhase,
}
```

And a controller that can:

- lazily create or fetch the panel
- cache current phase + metrics
- apply compact / expanded / hidden transitions

- [ ] **Step 3: Implement the macOS `tauri-nspanel` adapter**

In `panel.rs`, use `tauri-nspanel` to:

- create the `notification-overlay` panel
- set non-activating / always-on-top / all-spaces / fullscreen-auxiliary behavior
- reuse the panel across notifications

On non-macOS, keep a no-op or existing-window fallback so the crate still compiles.

- [ ] **Step 4: Run the focused Rust test to verify it passes**

Run: `cargo test --test notification_panel_controller`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/providers/notification/panel.rs src-tauri/src/providers/notification/mod.rs src-tauri/src/providers/notification/types.rs src-tauri/tests/notification_panel_controller.rs
git commit -m "feat: add tauri nspanel panel controller"
```

## Task 3: Refactor the Notification Provider and App Setup Around the Panel Layer

**Files:**
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/notification_overlay_geometry.rs`

- [ ] **Step 1: Write/update failing tests for provider-to-panel integration**

Add or extend tests to lock:

- overlay provider caches the active overlay metrics for later phase switching
- sending a notification prepares compact + expanded panel frames
- app setup pre-creates the panel instead of patching a generic window

- [ ] **Step 2: Run the targeted Rust tests to verify failure**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: FAIL on outdated initialization or panel ownership assertions.

- [ ] **Step 3: Refactor `overlay.rs`**

Make `overlay.rs` responsible only for:

- resolving notch-aware metrics
- building the view model
- delegating panel actions to `panel.rs`
- caching the active notification's panel state for later phase sync

Delete the manual AppKit patch path from `overlay.rs`.

- [ ] **Step 4: Refactor `lib.rs` setup**

During app setup:

- pre-create the overlay panel via `panel.rs`
- stop calling the old manual window patch path

- [ ] **Step 5: Run Rust tests again**

Run:

```bash
cargo test --test notification_panel_controller
cargo test --test notification_overlay_geometry
```

Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/providers/notification/overlay.rs src-tauri/src/lib.rs src-tauri/tests/notification_overlay_geometry.rs
git commit -m "refactor: move overlay lifecycle to panel layer"
```

## Task 4: Move Boundary Geometry Sync from Frontend Window APIs to Backend Panel Commands

**Files:**
- Modify: `src-tauri/src/commands/notification.rs`
- Modify: `src/lib/tauri.ts`
- Modify: `src/lib/tauri.test.ts`
- Modify: `src/overlay/use-overlay-notification.ts`
- Modify: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write failing frontend tests for backend-owned panel sync**

Update `src/overlay/overlay-app.test.tsx` so it expects:

- no direct calls to `getCurrentWindow().setSize/setPosition/show/hide`
- a backend panel sync invocation on `compact-idle`
- a backend panel sync invocation on `expanding` / `expanded`
- a backend panel hide invocation when the notification becomes hidden

- [ ] **Step 2: Write a failing wrapper test**

In `src/lib/tauri.test.ts`, add tests for new wrappers such as:

```ts
await tauri.syncNotificationOverlayPanel("compact");
await tauri.syncNotificationOverlayPanel("expanded");
await tauri.hideNotificationOverlayPanel();
```

Expected invoke names should be fixed and explicit.

- [ ] **Step 3: Run frontend tests to verify failure**

Run:

```bash
pnpm test src/overlay/overlay-app.test.tsx
pnpm test src/lib/tauri.test.ts
```

Expected: FAIL because the frontend still mutates the current window directly and the wrapper commands do not exist yet.

- [ ] **Step 4: Add the Tauri command**

In `src-tauri/src/commands/notification.rs`, add a command that takes a panel phase enum such as:

```rust
pub enum OverlayPanelCommandPhase {
    Compact,
    Expanded,
    Hidden,
}
```

The command should:

- look up the cached active overlay panel state
- switch the panel to the requested boundary frame
- hide when phase is `Hidden`

- [ ] **Step 5: Update frontend hooks and wrappers**

Replace direct `getCurrentWindow()` mutation in `src/overlay/use-overlay-notification.ts` with invoke wrappers from `src/lib/tauri.ts`.

- [ ] **Step 6: Run frontend tests again**

Run:

```bash
pnpm test src/overlay/overlay-app.test.tsx
pnpm test src/lib/tauri.test.ts
```

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/notification.rs src/lib/tauri.ts src/lib/tauri.test.ts src/overlay/use-overlay-notification.ts src/overlay/overlay-app.test.tsx
git commit -m "feat: sync overlay panel phases through backend commands"
```

## Task 5: Final Verification

**Files:**
- Verify: `src-tauri/src/providers/notification/panel.rs`
- Verify: `src-tauri/src/providers/notification/overlay.rs`
- Verify: `src-tauri/src/commands/notification.rs`
- Verify: `src/overlay/use-overlay-notification.ts`

- [ ] **Step 1: Run focused Rust overlay tests**

Run:

```bash
cargo test --test notification_panel_controller
cargo test --test notification_overlay_geometry
```

Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 2: Run existing Rust config regression**

Run: `cargo test --test config_services`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 3: Run focused frontend tests**

Run:

```bash
pnpm test src/overlay/overlay-app.test.tsx
pnpm test src/lib/tauri.test.ts
pnpm test src/lib/config-tauri.test.ts
```

Expected: PASS

- [ ] **Step 4: Run a production build**

Run: `pnpm build`
Expected: PASS

- [ ] **Step 5: Review diff for scope**

Run:

```bash
git diff -- src-tauri/Cargo.toml src-tauri/src/providers/notification src-tauri/src/commands/notification.rs src-tauri/src/lib.rs src-tauri/tests src/lib/tauri.ts src/lib/tauri.test.ts src/overlay/use-overlay-notification.ts src/overlay/overlay-app.test.tsx
```

Expected: only panel-first overlay refactor, panel sync bridge, and related tests.
