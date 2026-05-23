# Overlay Panel Motion Tuning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the overlay panel feel more like Dynamic Island by emphasizing expansion on trigger and suction on collapse without changing the persistent panel lifecycle.

**Architecture:** Keep the existing overlay phase machine and window sync flow. Split motion timing by surface vs. content so shell geometry leads the expansion while body content enters and exits with stronger staged offsets and opacity changes.

**Tech Stack:** React 19, Framer Motion, Vitest, jsdom

---

### Task 1: Lock the desired motion semantics in tests

**Files:**
- Modify: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write the failing test**

Add expectations that expansion and collapse body motion differ by phase, and that collapse uses a stronger exit offset than compact idle.

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- src/overlay/overlay-app.test.tsx`
Expected: FAIL on the new motion expectations.

- [ ] **Step 3: Write minimal implementation**

Update overlay motion helpers to return distinct transitions/motion values for shell and body phases.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- src/overlay/overlay-app.test.tsx`
Expected: PASS

### Task 2: Tune staged expansion and collapse motion

**Files:**
- Modify: `src/overlay/overlay-model.ts`
- Modify: `src/overlay/overlay-app.tsx`
- Test: `src/overlay/overlay-app.test.tsx`

- [ ] **Step 1: Write the failing test**

Add assertions covering shell/content transitions used during `expanding`, `expanded`, and `collapse-delay`.

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- src/overlay/overlay-app.test.tsx`
Expected: FAIL because all layers still share one transition profile.

- [ ] **Step 3: Write minimal implementation**

Introduce phase-aware shell/content transition helpers and wire them into the overlay island motion tree.

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- src/overlay/overlay-app.test.tsx`
Expected: PASS

### Task 3: Verify related overlay commands stay green

**Files:**
- Test: `src/lib/tauri.test.ts`
- Test: `src/lib/tauri-capabilities.test.ts`

- [ ] **Step 1: Run focused verification**

Run: `npm run test -- src/overlay/overlay-app.test.tsx src/lib/tauri.test.ts src/lib/tauri-capabilities.test.ts`
Expected: PASS
