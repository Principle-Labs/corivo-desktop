# Notch-Aware Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement notch-aware notification overlay geometry for Corivo so notifications target the foreground window's screen, preserve the current simulated island on non-notch displays, and anchor a true-notch island more credibly on notched displays.

**Architecture:** Keep the existing single-provider, single-window overlay stack. Add a thin Rust `screen_metrics` layer that selects the target monitor, computes notch/safe-area metrics, and emits compact/expanded window geometry; then update the React overlay to consume backend-owned geometry instead of inferring positions from `window.screen`.

**Tech Stack:** Rust, Tauri 2, objc2 AppKit/CoreGraphics, React 19, Framer Motion, Vitest, Cargo tests

---

## File Structure

- Create: `src-tauri/src/providers/notification/screen_metrics.rs`
  Responsibility: screen selection, notch detection, backend-owned compact/expanded geometry, pure testable helpers.
- Modify: `src-tauri/src/providers/notification/mod.rs`
  Responsibility: export `screen_metrics`.
- Modify: `src-tauri/src/providers/notification/types.rs`
  Responsibility: extend overlay view model with notch/screen/window geometry fields.
- Modify: `src-tauri/src/providers/notification/overlay.rs`
  Responsibility: build view model from `screen_metrics` and use backend-owned window geometry when creating/repositioning the overlay window.
- Modify: `src/lib/types.ts`
  Responsibility: mirror the overlay view model shape for the frontend.
- Modify: `src/overlay/overlay-model.ts`
  Responsibility: resolve window geometry from backend-provided compact/expanded bounds and add notch-aware presentation helpers.
- Modify: `src/overlay/overlay-app.tsx`
  Responsibility: adapt island layout to notch-aware metrics while preserving the current simulated-island behavior.
- Modify: `src/overlay/overlay-app.test.tsx`
  Responsibility: lock frontend geometry ownership and notch-aware layout rules.
- Create: `src-tauri/tests/notification_overlay_geometry.rs`
  Responsibility: lock backend screen selection and geometry fallback behavior.

## Task 1: Lock Backend Geometry Behavior with Failing Tests

**Files:**
- Create: `src-tauri/tests/notification_overlay_geometry.rs`
- Test: `src-tauri/src/providers/notification/screen_metrics.rs`

- [ ] **Step 1: Write failing tests for screen selection and geometry fallbacks**

Add pure-Rust tests covering:

- foreground window center chooses the matching secondary screen
- overlap fallback chooses the screen with the largest intersection
- missing foreground window falls back to the main screen
- notched screens produce `anchor_mode = Notch` and widened compact geometry
- geometry failures fall back to simulated/default sizing

- [ ] **Step 2: Run the focused Rust test to verify it fails**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: FAIL because the test target or helper module does not exist yet.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/notification_overlay_geometry.rs
git commit -m "test: define notch-aware overlay geometry contract"
```

## Task 2: Implement Rust Screen Metrics and View Model Expansion

**Files:**
- Create: `src-tauri/src/providers/notification/screen_metrics.rs`
- Modify: `src-tauri/src/providers/notification/mod.rs`
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Test: `src-tauri/tests/notification_overlay_geometry.rs`

- [ ] **Step 1: Implement a testable screen geometry model**

Add pure structs/functions for:

- target screen selection from screen frames + optional foreground window bounds
- notch metrics (`has_notch`, `anchor_mode`, `top_safe_height_px`, `notch_width_px`)
- compact/expanded window geometry generation

- [ ] **Step 2: Add platform adapters**

On macOS:
- inspect on-screen app windows via CoreGraphics
- detect notch metrics from AppKit screen safe-area / auxiliary areas

On non-macOS:
- return simulated metrics and main monitor fallback

- [ ] **Step 3: Extend `OverlayNotificationViewModel`**

Add:

- `has_notch`
- `anchor_mode`
- `screen_width_px`
- `screen_height_px`
- `top_safe_height_px`
- `notch_width_px`
- `compact_window_{x,y,width,height}`
- `expanded_window_{x,y,width,height}`

- [ ] **Step 4: Update overlay provider**

Use `screen_metrics` to:

- compute per-notification geometry
- position the initial overlay window from compact geometry
- emit the expanded/compact geometry to the frontend

- [ ] **Step 5: Run the focused Rust tests**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/providers/notification/screen_metrics.rs src-tauri/src/providers/notification/mod.rs src-tauri/src/providers/notification/types.rs src-tauri/src/providers/notification/overlay.rs src-tauri/tests/notification_overlay_geometry.rs
git commit -m "feat: add notch-aware overlay screen metrics"
```

## Task 3: Lock Frontend Geometry Ownership with Failing Tests

**Files:**
- Modify: `src/overlay/overlay-app.test.tsx`
- Modify: `src/lib/types.ts`

- [ ] **Step 1: Write failing tests for backend-owned window geometry**

Add tests asserting:

- compact sync uses `compact_window_*` exactly
- expanded/collapse-delay sync uses `expanded_window_*` exactly
- no code path depends on `window.screen.width`

- [ ] **Step 2: Write failing tests for notch-aware presentation**

Add tests asserting:

- `has_notch = true` yields notch-aware compact presentation data
- `has_notch = false` preserves the current simulated island path/layout

- [ ] **Step 3: Run the focused frontend test to verify failure**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: FAIL on the new geometry/layout assertions.

- [ ] **Step 4: Commit**

```bash
git add src/overlay/overlay-app.test.tsx src/lib/types.ts
git commit -m "test: define frontend notch-aware overlay contract"
```

## Task 4: Implement Frontend Geometry Consumption and Notch-Aware Layout

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/overlay/overlay-model.ts`
- Modify: `src/overlay/use-overlay-notification.ts`
- Modify: `src/overlay/overlay-app.tsx`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Mirror the new backend fields in TypeScript**

Update `OverlayNotificationViewModel` in `src/lib/types.ts`.

- [ ] **Step 2: Move window geometry ownership fully to backend data**

Update `resolveWindowGeometry` to choose between:

- compact window geometry for `compact-idle`
- expanded window geometry for `expanding`, `expanded`, `collapse-delay`

- [ ] **Step 3: Add notch-aware presentation helpers**

Use `has_notch`, `anchor_mode`, `notch_width_px`, and `top_safe_height_px` to drive:

- compact width/height presentation
- shell/top-extension tuning
- headline/content spacing that preserves a stronger center gap on true-notch screens

- [ ] **Step 4: Run focused frontend tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/lib/types.ts src/overlay/overlay-model.ts src/overlay/use-overlay-notification.ts src/overlay/overlay-app.tsx src/overlay/overlay-app.test.tsx
git commit -m "feat: render notch-aware overlay geometry"
```

## Task 5: Final Verification

**Files:**
- Verify: `src-tauri/src/providers/notification/overlay.rs`
- Verify: `src-tauri/src/providers/notification/screen_metrics.rs`
- Verify: `src/overlay/overlay-model.ts`
- Verify: `src/overlay/overlay-app.tsx`

- [ ] **Step 1: Run focused Rust geometry tests**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 2: Run existing Rust config regression**

Run: `cargo test --test config_services`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 3: Run focused frontend tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 4: Run related frontend config test**

Run: `pnpm test src/lib/config-tauri.test.ts`
Expected: PASS

- [ ] **Step 5: Run a production build**

Run: `pnpm build`
Expected: PASS

- [ ] **Step 6: Review diff for scope**

Run:

```bash
git diff -- src-tauri/src/providers/notification src-tauri/tests/notification_overlay_geometry.rs src/overlay src/lib/types.ts
```

Expected: only notch-aware overlay geometry and related test updates.
