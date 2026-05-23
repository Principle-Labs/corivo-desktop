# Native Hover Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move overlay hover ownership from the frontend DOM to the macOS native panel so notification expansion remains reliable even when Corivo is unfocused.

**Architecture:** Add a Rust-owned overlay session state machine and native hover bridge in the panel adapter, emit authoritative UI phase events to the renderer, and downgrade the frontend to a pure notification/phase subscriber. Reuse the current compact/expanded geometry model and single `notification-overlay` panel instead of rebuilding the overlay container.

**Tech Stack:** Rust, Tauri 2, tauri-nspanel, objc2 AppKit bindings, React, Vitest

---

### Task 1: Define the backend-owned phase contract

**Files:**
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src/lib/types.ts`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Add the Rust UI phase and phase event types**

Add a serializable Rust enum for:

```rust
pub enum OverlayUiPhase {
    Hidden,
    CompactIdle,
    Expanding,
    Expanded,
    CollapseDelay,
}
```

Add a serializable Rust event struct:

```rust
pub struct OverlayPhaseChangedEvent {
    pub sequence: u64,
    pub phase: OverlayUiPhase,
}
```

- [ ] **Step 2: Mirror the phase contract in the frontend types**

Add to `src/lib/types.ts`:

```ts
export type OverlayUiPhase =
  | "hidden"
  | "compact_idle"
  | "expanding"
  | "expanded"
  | "collapse_delay";

export interface OverlayPhaseChangedEvent {
  sequence: number;
  phase: OverlayUiPhase;
}
```

- [ ] **Step 3: Update the existing overlay tests to compile against the new types**

Replace any assumptions that the frontend owns phase transitions. Keep the test file compiling before deeper behavior changes.

- [ ] **Step 4: Run the focused frontend test to verify the type changes compile**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: existing hover tests still fail later for behavior, but the suite compiles with the new type contract.

### Task 2: Add a Rust overlay session state machine

**Files:**
- Create: `src-tauri/src/providers/notification/overlay_session.rs`
- Modify: `src-tauri/src/providers/notification/mod.rs`
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Test: `src-tauri/tests/notification_overlay_session.rs`

- [ ] **Step 1: Write the failing Rust session-state tests**

Create `src-tauri/tests/notification_overlay_session.rs` covering:

```rust
#[test]
fn new_notification_starts_compact_idle() {}

#[test]
fn compact_hover_enters_expanding_then_expanded() {}

#[test]
fn expanded_hover_exit_enters_collapse_delay() {}

#[test]
fn collapse_delay_reenter_returns_to_expanded() {}

#[test]
fn lifetime_expiry_hides_when_compact() {}

#[test]
fn lifetime_expiry_routes_expanded_to_collapse_delay_then_hidden() {}
```

- [ ] **Step 2: Run the new Rust test file and verify it fails**

Run: `cargo test --test notification_overlay_session`

Expected: FAIL because the session module and state machine do not exist yet.

- [ ] **Step 3: Implement the minimal overlay session module**

Model:

- current notification sequence
- current `OverlayUiPhase`
- `expired` flag
- pending timers / deadlines abstractions that can be tested deterministically

Expose methods like:

```rust
fn start_notification(...)
fn on_hover_enter(...)
fn on_hover_exit(...)
fn on_animation_elapsed(...)
fn on_collapse_delay_elapsed(...)
fn on_lifetime_elapsed(...)
```

- [ ] **Step 4: Re-export the new session module**

Update `src-tauri/src/providers/notification/mod.rs` to expose `overlay_session`.

- [ ] **Step 5: Run the Rust session tests and make them pass**

Run: `cargo test --test notification_overlay_session`

Expected: PASS

### Task 3: Extend the panel adapter with native hover callbacks

**Files:**
- Modify: `src-tauri/src/providers/notification/panel.rs`
- Test: `src-tauri/tests/notification_overlay_geometry.rs`

- [ ] **Step 1: Write a failing panel-controller test for hover tracking bookkeeping**

Add tests in `src-tauri/tests/notification_overlay_geometry.rs` that verify:

- compact bounds registration updates tracking bounds
- expanded bounds registration updates tracking bounds
- hidden state clears tracking

Use the recording adapter pattern already present in the file.

- [ ] **Step 2: Run the geometry test file and verify the new cases fail**

Run: `cargo test --test notification_overlay_geometry`

Expected: FAIL because the adapter trait has no hover tracking hooks yet.

- [ ] **Step 3: Add hover-tracking methods to `OverlayPanelAdapter`**

Extend the trait with methods for:

```rust
fn install_hover_tracking(...)
fn update_hover_tracking(...)
fn clear_hover_tracking(...)
```

Keep no-op implementations simple on non-macOS builds.

- [ ] **Step 4: Add macOS-native hover tracking plumbing**

In `panel.rs`, on macOS:

- install tracking on panel creation
- update tracking when the active phase frame changes
- clear tracking on hide

Keep the first version rectangle-based and tied to current panel bounds.

- [ ] **Step 5: Run the geometry test file and make the new bookkeeping tests pass**

Run: `cargo test --test notification_overlay_geometry`

Expected: PASS

### Task 4: Make Rust the owner of phase and frame synchronization

**Files:**
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src-tauri/src/commands/notification.rs`
- Test: `src-tauri/tests/notification_overlay_session.rs`

- [ ] **Step 1: Add a backend event name for phase changes**

Define an event constant like:

```rust
pub const OVERLAY_PHASE_EVENT: &str = "overlay-phase-changed";
```

- [ ] **Step 2: Thread the session state machine into notification send flow**

When `send()` runs:

- resolve metrics
- prepare compact/expanded bounds
- show compact panel
- start a new overlay session in `compact_idle`
- emit notification payload
- emit phase event

- [ ] **Step 3: Wire hover enter/exit and timer callbacks to session transitions**

Route:

- native hover enter -> session transition -> panel phase sync -> phase event emit
- native hover exit -> session transition -> panel phase sync -> phase event emit
- animation/lifetime/collapse timers -> same path

- [ ] **Step 4: Keep `sync_notification_overlay_panel` only for debugging**

Do not remove the command yet if tests/tools still use it, but stop relying on it in the formal interaction path.

- [ ] **Step 5: Run the Rust session tests again**

Run: `cargo test --test notification_overlay_session`

Expected: PASS with real integration points in place.

### Task 5: Downgrade the frontend from state owner to phase subscriber

**Files:**
- Modify: `src/overlay/use-overlay-notification.ts`
- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/lib/tauri.ts`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write failing frontend tests for backend-driven phases**

Add or replace tests so they assert:

- `overlay-notification` sets the active notification
- `overlay-phase-changed` updates the rendered state
- the component no longer depends on `mouseover` / `mouseout` to progress phases

- [ ] **Step 2: Run the focused frontend test and verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: FAIL because the hooks still own timers and DOM hover state.

- [ ] **Step 3: Refactor `use-overlay-notification.ts` into event subscriptions**

Implement:

- one hook for notification feed
- one hook for authoritative phase feed

Remove:

- frontend lifetime timer
- frontend animation timer
- frontend collapse timer
- hover-driven `setPhase`

- [ ] **Step 4: Remove DOM hover handlers from the overlay component**

Delete `onMouseEnter` / `onMouseLeave` from `OverlayNotificationStage`.

- [ ] **Step 5: Keep panel sync invokes out of the main rendering path**

Delete or stop using the automatic `syncNotificationOverlayPanel` / `hideNotificationOverlayPanel` effect for normal UI flow.

- [ ] **Step 6: Run the frontend test and make it pass**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: PASS

### Task 6: Verify the end-to-end focused test surface

**Files:**
- Verify: `src-tauri/src/providers/notification/panel.rs`
- Verify: `src-tauri/src/providers/notification/overlay.rs`
- Verify: `src-tauri/src/providers/notification/overlay_session.rs`
- Verify: `src/overlay/use-overlay-notification.ts`
- Verify: `src/overlay/overlay-app.tsx`
- Verify: `src/lib/types.ts`
- Verify: `src-tauri/tests/notification_overlay_session.rs`
- Verify: `src-tauri/tests/notification_overlay_geometry.rs`
- Verify: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Run the Rust overlay session tests**

Run: `cargo test --test notification_overlay_session`

Expected: PASS

- [ ] **Step 2: Run the Rust geometry tests**

Run: `cargo test --test notification_overlay_geometry`

Expected: PASS

- [ ] **Step 3: Run the focused frontend overlay test**

Run: `pnpm test src/overlay/overlay-app.test.tsx`

Expected: PASS

- [ ] **Step 4: Inspect the final diff for unintended scope creep**

Run: `git diff -- src-tauri/src/providers/notification/panel.rs src-tauri/src/providers/notification/overlay.rs src-tauri/src/providers/notification/overlay_session.rs src-tauri/src/providers/notification/types.rs src/overlay/use-overlay-notification.ts src/overlay/overlay-app.tsx src/lib/types.ts src-tauri/tests/notification_overlay_session.rs src-tauri/tests/notification_overlay_geometry.rs src/overlay/overlay-app.test.tsx docs/superpowers/specs/2026-04-15-native-hover-overlay-design.md docs/superpowers/plans/2026-04-15-native-hover-overlay.md`

Expected: only native-hover ownership, phase event, and related tests/docs changed
