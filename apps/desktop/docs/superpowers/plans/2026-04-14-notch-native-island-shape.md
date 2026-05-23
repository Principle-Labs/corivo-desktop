# Notch Native Island Shape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the top notification island feel notch-native by keeping the top edge hard and stable, reshaping the expanded state into a hanging panel, and preserving smooth motion with window-boundary-only native resizing.

**Architecture:** Keep the existing single-window compact/expanded state machine. The Tauri window continues to resize only at compact/expanded boundaries, while the island’s visual form is driven entirely in `src/overlay/overlay-app.tsx` via Framer Motion. Static island skin stays in overlay-specific CSS, while animated geometry and corner ownership stay in motion state.

**Tech Stack:** React 19, Framer Motion, Vitest, Tauri window API, Tailwind base styles + overlay CSS

---

## File Structure

- Modify: `src/overlay/overlay-app.tsx`
  Responsibility: Own island state transitions, motion geometry, top-edge stability, and render structure for compact vs expanded shapes.
- Modify: `src/overlay/overlay-app.test.tsx`
  Responsibility: Lock shape behavior with failing tests first, especially top-corner invariants, window resize cadence, and expanded-state DOM shape cues.
- Modify: `src/styles/overlay/island.css`
  Responsibility: Hold the static notch-native visual skin for the island and expanded panel.
- Modify: `src-tauri/src/domain/config.rs`
  Responsibility: Tune default compact/expanded geometry only if the new shape needs a different baseline.
- Modify: `src-tauri/tests/config_services.rs`
  Responsibility: Keep Rust default-config assertions aligned if geometry defaults change.
- Verify: `src/overlay/main.tsx`
  Responsibility: Continue importing overlay-specific styles only; no behavior change expected.

## Task 1: Lock the Shape Contract in Frontend Tests

**Files:**
- Modify: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write a failing test for notch-native compact shape**

Add a test that renders the compact island and asserts:

```tsx
expect(islandElement()?.style.borderTopLeftRadius).toBe("0px");
expect(islandElement()?.style.borderTopRightRadius).toBe("0px");
expect(stageElement()?.dataset.state).toBe("compact-enter");
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: FAIL because current compact motion or DOM does not yet satisfy the full notch-native shape contract.

- [ ] **Step 3: Write a failing test for expanded hanging-panel behavior**

Add a test that hovers into expanded state and asserts:

```tsx
expect(stageElement()?.dataset.state).toBe("expanded");
expect(islandElement()?.style.borderTopLeftRadius).toBe("0px");
expect(islandElement()?.style.borderTopRightRadius).toBe("0px");
expect(bodyElement()).not.toBeNull();
```

Also assert that the window only resized once at enter-boundary and not continuously during steady expanded state.

- [ ] **Step 4: Run test to verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: FAIL on the new expanded-shape assertions before implementation.

- [ ] **Step 5: Commit**

```bash
git add src/overlay/overlay-app.test.tsx
git commit -m "test: define notch-native island shape contract"
```

## Task 2: Rework Motion Geometry for Hard Top / Soft Bottom

**Files:**
- Modify: `src/overlay/overlay-app.tsx`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Implement compact and expanded motion objects with explicit corner ownership**

Update `islandMotion` so that:

```tsx
borderTopLeftRadius: 0
borderTopRightRadius: 0
borderBottomLeftRadius: compactOrExpandedRadius
borderBottomRightRadius: compactOrExpandedRadius
```

Keep top-edge geometry stable across all phases.

- [ ] **Step 2: Preserve boundary-only native window resizing**

Keep `resolveWindowGeometry` and `sameWindowGeometry` behavior so the native window only changes when switching compact/expanded bounds, not every phase tick.

- [ ] **Step 3: Run focused tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS for all shape and window-resize cadence tests.

- [ ] **Step 4: Refactor only if needed**

If the motion object has duplicated geometry, extract compact and expanded helpers inside `overlay-app.tsx`. Do not introduce new files unless the component becomes hard to read.

- [ ] **Step 5: Commit**

```bash
git add src/overlay/overlay-app.tsx src/overlay/overlay-app.test.tsx
git commit -m "feat: animate notch-native island geometry"
```

## Task 3: Turn the Expanded State into a Hanging Panel

**Files:**
- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/styles/overlay/island.css`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write a failing test for hanging-panel visual structure**

Add a test that expands the island and asserts the body stays mounted under the compact header while the state is `expanded`, and that the top corners remain square after expansion settles.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: FAIL if the expanded state still reads as a large pill instead of a header + hanging panel.

- [ ] **Step 3: Adjust render structure and styling**

Update the render/CSS so the expanded state visually reads as:

```text
top hard bar
content panel growing downward
soft rounded lower edge
```

Prefer:
- a stable compact header row at the top
- content padding and spacing that shift weight downward
- bottom-heavy shadowing

Avoid:
- fully symmetric capsule proportions
- extra decoration above the top edge

- [ ] **Step 4: Run frontend tests again**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/overlay/overlay-app.tsx src/styles/overlay/island.css src/overlay/overlay-app.test.tsx
git commit -m "feat: shape expanded island as hanging panel"
```

## Task 4: Tune Default Geometry If the New Shape Needs It

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src-tauri/tests/config_services.rs`

- [ ] **Step 1: Decide whether the hanging-panel shape needs narrower compact or rebalanced expanded bounds**

Check whether the current defaults:

```rust
compact_width_px: 248,
compact_height_px: 52,
expanded_width_px: 420,
expanded_height_px: 156,
```

still fit the new notch-native design.

- [ ] **Step 2: If changing defaults, write the failing Rust assertions first**

Update `src-tauri/tests/config_services.rs` expected values before changing `config.rs`.

- [ ] **Step 3: Run Rust config test to verify failure**

Run: `cargo test --test config_services`
Workdir: `src-tauri`
Expected: FAIL on the changed geometry assertions if defaults are being tuned.

- [ ] **Step 4: Apply minimal config default updates**

Change only the default geometry values required by the visual design. Do not widen scope into provider logic or migration rules.

- [ ] **Step 5: Run Rust config test to verify pass**

Run: `cargo test --test config_services`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/domain/config.rs src-tauri/tests/config_services.rs
git commit -m "chore: tune default notch-native island geometry"
```

## Task 5: Final Verification

**Files:**
- Verify: `src/overlay/overlay-app.tsx`
- Verify: `src/styles/overlay/island.css`
- Verify: `src-tauri/src/domain/config.rs`

- [ ] **Step 1: Run frontend shape tests**

Run: `pnpm test src/overlay/overlay-app.test.tsx`
Expected: PASS

- [ ] **Step 2: Run capability/config regression tests**

Run: `pnpm test src/lib/tauri-capabilities.test.ts`
Expected: PASS

- [ ] **Step 3: Run Rust config tests**

Run: `cargo test --test config_services`
Workdir: `src-tauri`
Expected: PASS

- [ ] **Step 4: Run production build**

Run: `pnpm build`
Expected: PASS and emit both `index.html` and `notification-overlay.html`

- [ ] **Step 5: Review diff for scope**

Run:

```bash
git diff -- src/overlay/overlay-app.tsx src/overlay/overlay-app.test.tsx src/styles/overlay/island.css src-tauri/src/domain/config.rs src-tauri/tests/config_services.rs
```

Expected: only notch-native island shape, motion, and optional geometry-default changes.

- [ ] **Step 6: Commit final polish if needed**

```bash
git add src/overlay/overlay-app.tsx src/overlay/overlay-app.test.tsx src/styles/overlay/island.css src-tauri/src/domain/config.rs src-tauri/tests/config_services.rs
git commit -m "feat: finalize notch-native notification island"
```
