# Notch Split / Expanded Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compact notification island split content around the notch while letting the expanded state show more body content without losing the top notch gap.

**Architecture:** Keep the current single overlay window, current stage machine, and current backend-owned window geometry. Extend the Rust notch metrics contract with an explicit headline gap and stronger compact width minimums, then move the React overlay from padding-based notch avoidance to a real three-column headline layout with a full-width expanded body.

**Tech Stack:** Rust, Tauri 2, React 19, Framer Motion, CSS, Cargo tests, Vitest, pnpm

---

## File Structure

- Modify: `src-tauri/src/providers/notification/screen_metrics.rs`
  Responsibility: define the notch gap contract and update compact / expanded width calculations for true-notch screens.
- Modify: `src-tauri/src/providers/notification/types.rs`
  Responsibility: add the frontend-facing notch gap field to `OverlayNotificationViewModel`.
- Modify: `src-tauri/src/providers/notification/overlay.rs`
  Responsibility: pass the new notch gap metric from backend geometry into the emitted overlay view model.
- Modify: `src-tauri/tests/notification_overlay_geometry.rs`
  Responsibility: lock the new compact-width and expanded-width notch contract before implementation.
- Modify: `src/lib/types.ts`
  Responsibility: mirror the new overlay view model field for the frontend.
- Modify: `src/overlay/overlay-model.ts`
  Responsibility: add presentation helpers for split headline / expanded body layout and keep geometry phase selection centralized.
- Modify: `src/overlay/overlay-app.tsx`
  Responsibility: render notch-aware compact and expanded headline layouts explicitly instead of relying on padding.
- Modify: `src/styles/overlay/island.css`
  Responsibility: define the three-column headline grid, left/right alignment, headline gap variable, and full-width expanded body layout.
- Modify: `src/overlay/overlay-app.test.tsx`
  Responsibility: lock the compact split headline, expanded notch-preserving headline, and full-width expanded body behavior.

## Task 1: Lock the Backend Notch-Gap Contract with Failing Tests

**Files:**
- Modify: `src-tauri/tests/notification_overlay_geometry.rs`
- Test: `src-tauri/src/providers/notification/screen_metrics.rs`

- [ ] **Step 1: Add a failing test for true-notch compact readable zones**

Add a test that builds `OverlayGeometryInput` with a true-notch screen and asserts the resulting metrics satisfy a stronger compact contract:

```rust
assert!(metrics.has_notch);
assert_eq!(metrics.anchor_mode, AnchorMode::Notch);
assert!(metrics.notch_gap_px.unwrap_or(0) >= 210);
assert!(metrics.compact_width_px >= metrics.notch_gap_px.unwrap() + 160);
```

Use a fixed fixture so the contract is deterministic.

- [ ] **Step 2: Add a failing test for expanded width growth without top drift**

Add a second test that asserts:

```rust
assert!(metrics.expanded_width_px > metrics.compact_width_px);
assert_eq!(metrics.compact_window_y, metrics.expanded_window_y);
assert_eq!(metrics.notch_gap_px, Some(210));
```

This keeps the expanded state wider while preserving the same top anchor and same notch-gap language.

- [ ] **Step 3: Run the focused Rust test target to verify failure**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: FAIL because `notch_gap_px` and the stronger compact-width contract do not exist yet.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/notification_overlay_geometry.rs
git commit -m "test: define split notch overlay geometry contract"
```

## Task 2: Implement Rust Notch-Gap Metrics and Width Rules

**Files:**
- Modify: `src-tauri/src/providers/notification/screen_metrics.rs`
- Modify: `src-tauri/src/providers/notification/types.rs`
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Test: `src-tauri/tests/notification_overlay_geometry.rs`

- [ ] **Step 1: Extend `OverlayMetrics` with an explicit notch gap field**

Add `notch_gap_px: Option<u32>` in `screen_metrics.rs` and `OverlayNotificationViewModel` so the frontend consumes a stable contract instead of inferring the gap from padding.

Minimal shape:

```rust
pub struct OverlayMetrics {
    // existing fields...
    pub notch_width_px: Option<u32>,
    pub notch_gap_px: Option<u32>,
}
```

- [ ] **Step 2: Encode the compact minimum width as left zone + notch gap + right zone**

Define named constants in `screen_metrics.rs`, for example:

```rust
const COMPACT_NOTCH_SIDE_ZONE_PX: u32 = 80;
const EXPANDED_NOTCH_SIDE_ZONE_PX: u32 = 108;
```

Then compute:

```rust
let notch_gap_px = valid_notch.then_some(input.screen.notch_width_px.unwrap_or_default());
let compact_min_width_px =
    notch_gap_px.unwrap_or(0) + COMPACT_NOTCH_SIDE_ZONE_PX * 2;
```

Clamp that against the screen bounds exactly once. Keep YAGNI: no per-device tables.

- [ ] **Step 3: Update expanded width growth to keep the same gap but reserve more side room**

Use compact width as the floor, then ensure expanded width has visibly larger readable zones:

```rust
let expanded_min_width_px =
    notch_gap_px.unwrap_or(0) + EXPANDED_NOTCH_SIDE_ZONE_PX * 2;
let expanded_width_px = input
    .expanded_width_px
    .max(compact_width_px.saturating_add(EXPANDED_MIN_GROWTH_PX))
    .max(expanded_min_width_px)
    .min(expanded_content_max_width.max(1));
```

Do not change screen selection, safe-area lookup, or fallback rules.

- [ ] **Step 4: Emit the new field from the overlay provider**

In `src-tauri/src/providers/notification/overlay.rs`, pass `metrics.notch_gap_px` through `build_view_model` so the frontend can render the three-column layout with a backend-owned gap value.

- [ ] **Step 5: Run the focused Rust tests**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/providers/notification/screen_metrics.rs src-tauri/src/providers/notification/types.rs src-tauri/src/providers/notification/overlay.rs src-tauri/tests/notification_overlay_geometry.rs
git commit -m "feat: add split notch overlay metrics"
```

## Task 3: Lock the Frontend Layout Contract with Failing Tests

**Files:**
- Modify: `src/overlay/overlay-app.test.tsx`
- Modify: `src/lib/types.ts`
- Test: `src/overlay/overlay-app.tsx`

- [ ] **Step 1: Mirror `notch_gap_px` in the TypeScript test fixture**

Add the field to `OverlayNotificationViewModel` in `src/lib/types.ts` and set it in the test helper:

```ts
notch_width_px: 210,
notch_gap_px: 210,
```

Keep simulated fixtures at `null`.

- [ ] **Step 2: Add a failing compact-layout test for the split headline**

Add a test that renders a true-notch notification and asserts:

```ts
expect(headlineElement()?.dataset.layout).toBe("split");
expect(headlineElement()?.style.getPropertyValue("--island-notch-gap")).toBe("210px");
expect(container?.querySelector("[data-testid='overlay-headline-left']")).not.toBeNull();
expect(container?.querySelector("[data-testid='overlay-headline-gap']")).not.toBeNull();
expect(container?.querySelector("[data-testid='overlay-headline-right']")).not.toBeNull();
```

- [ ] **Step 3: Add a failing expanded-layout test for preserved gap + full-width body**

Expand the notification and assert:

```ts
expect(headlineElement()?.dataset.layout).toBe("split-expanded");
expect(bodyElement()?.dataset.layout).toBe("full-width");
```

Also assert simulated mode keeps the current simple headline:

```ts
expect(headlineElement()?.dataset.layout).toBe("start");
```

- [ ] **Step 4: Run the focused frontend test to verify failure**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: FAIL on the new split-headline and expanded-body assertions.

- [ ] **Step 5: Commit**

```bash
git add src/lib/types.ts src/overlay/overlay-app.test.tsx
git commit -m "test: define split notch overlay layout contract"
```

## Task 4: Implement the Split Headline and Expanded Body Layout

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/overlay/overlay-model.ts`
- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/styles/overlay/island.css`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Add layout helpers in `overlay-model.ts`**

Introduce explicit layout helpers instead of overloading padding:

```ts
export function getOverlayHeadlineLayout(
  phase: OverlayPhase,
  notchAware: boolean,
) {
  if (!notchAware) return "start";
  if (phase === "expanded" || phase === "collapse-delay") return "split-expanded";
  if (phase === "compact-idle") return "split";
  return "center";
}

export function getOverlayBodyLayout(
  phase: OverlayPhase,
  notchAware: boolean,
) {
  if (notchAware && (phase === "expanded" || phase === "collapse-delay")) {
    return "full-width";
  }
  return "default";
}
```

- [ ] **Step 2: Render explicit left / gap / right headline slots in `overlay-app.tsx`**

Replace the current one-row icon/title markup with notch-aware conditional structure:

```tsx
<motion.div className="top-island__headline" data-layout={headlineLayout}>
  {headlineLayout.startsWith("split") ? (
    <>
      <div data-testid="overlay-headline-left">...</div>
      <div data-testid="overlay-headline-gap" aria-hidden="true" />
      <div data-testid="overlay-headline-right">...</div>
    </>
  ) : (
    <>
      <div className="top-island__icon" aria-hidden="true" />
      <div className="top-island__title">{activeNotification.title}</div>
    </>
  )}
</motion.div>
```

Keep the displayed content minimal:
- left: icon + title
- right: short status label such as `Notification`

Do not invent extra product copy or metadata fields in this task.

- [ ] **Step 3: Move notch layout styling into CSS**

In `src/styles/overlay/island.css`, add a CSS variable and grid-based layout:

```css
.top-island__headline[data-layout="split"],
.top-island__headline[data-layout="split-expanded"] {
  display: grid;
  grid-template-columns: minmax(0, 1fr) var(--island-notch-gap, 0px) minmax(0, 1fr);
}

.top-island__headline-left {
  justify-self: end;
}

.top-island__headline-right {
  justify-self: start;
}
```

Also mark the expanded body as full width:

```css
.top-island__body[data-layout="full-width"] {
  left: 0;
  right: 0;
  width: 100%;
}
```

Reuse existing spacing and motion where possible; do not re-theme the island.

- [ ] **Step 4: Feed the backend gap value into CSS variables**

In `useIslandPresentation`, add:

```ts
["--island-notch-gap" as string]: `${notification?.notch_gap_px ?? 0}px`,
```

Remove `--island-notch-leading-inset` from the critical path for true-notch layout.

- [ ] **Step 5: Run the focused frontend tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/lib/types.ts src/overlay/overlay-model.ts src/overlay/overlay-app.tsx src/styles/overlay/island.css src/overlay/overlay-app.test.tsx
git commit -m "feat: split notch headline and expanded body layout"
```

## Task 5: Final Verification

**Files:**
- Verify: `src-tauri/src/providers/notification/screen_metrics.rs`
- Verify: `src-tauri/src/providers/notification/overlay.rs`
- Verify: `src/lib/types.ts`
- Verify: `src/overlay/overlay-model.ts`
- Verify: `src/overlay/overlay-app.tsx`
- Verify: `src/styles/overlay/island.css`

- [ ] **Step 1: Re-run the focused Rust geometry test**

Run: `cargo test --test notification_overlay_geometry`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 2: Run the focused frontend overlay test**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 3: Run the related frontend config test**

Run: `pnpm test src/lib/config-tauri.test.ts`
Expected: PASS

- [ ] **Step 4: Run the production build**

Run: `pnpm build`
Expected: PASS

- [ ] **Step 5: Review the diff for scope**

Run:

```bash
git diff -- src-tauri/src/providers/notification/screen_metrics.rs src-tauri/src/providers/notification/types.rs src-tauri/src/providers/notification/overlay.rs src-tauri/tests/notification_overlay_geometry.rs src/lib/types.ts src/overlay/overlay-model.ts src/overlay/overlay-app.tsx src/styles/overlay/island.css src/overlay/overlay-app.test.tsx
```

Expected: only split-notch layout metrics, frontend layout changes, and related tests.

- [ ] **Step 6: Commit final verification or polish if needed**

```bash
git add src-tauri/src/providers/notification/screen_metrics.rs src-tauri/src/providers/notification/types.rs src-tauri/src/providers/notification/overlay.rs src-tauri/tests/notification_overlay_geometry.rs src/lib/types.ts src/overlay/overlay-model.ts src/overlay/overlay-app.tsx src/styles/overlay/island.css src/overlay/overlay-app.test.tsx
git commit -m "feat: finalize split notch notification overlay"
```
