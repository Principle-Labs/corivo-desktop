# macOS System Surface Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Corivo switch cleanly between foreground `Regular` mode and menu-bar-only `Accessory` mode on macOS so hiding the main window becomes a true background state and overlay panel activity does not promote the app back to foreground.

**Architecture:** Add a focused `macos_system_surface` service that owns the pure policy decision and the runtime activation-policy application. Keep the current menubar preview behavior, but route `show/hide/quit` and overlay visibility through the new owner so the app's system identity is no longer implied by ad-hoc `lib.rs` helpers.

**Tech Stack:** Rust, Tauri 2 macOS activation policy APIs, tauri-nspanel, cargo test

---

### Task 1: Lock The macOS Surface Policy With Pure Tests

**Files:**
- Create: `src-tauri/src/services/macos_system_surface.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/macos_system_surface.rs`

- [ ] **Step 1: Write the failing test**

Add pure unit tests for a policy function that maps visibility and lifecycle inputs to:

- `RegularForeground`
- `AccessoryBackground`

Required cases:

- visible main window => `RegularForeground`
- hidden main window with no other foreground surfaces => `AccessoryBackground`
- hidden main window with overlay visible only => `AccessoryBackground`
- hidden recovery launch => `AccessoryBackground`
- visible recovery flow => `RegularForeground`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test macos_system_surface --manifest-path src-tauri/Cargo.toml`
Expected: FAIL because the service and policy function do not exist yet.

- [ ] **Step 3: Write minimal implementation**

Add:

- `MacOSSystemSurfaceMode`
- `MacOSSystemSurfaceInputs`
- `resolve_macos_system_surface_mode(...)`

Keep it pure and testable.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test macos_system_surface --manifest-path src-tauri/Cargo.toml`
Expected: PASS

### Task 2: Route Main Window And Menubar Preview Through The Surface Owner

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/macos_system_surface.rs`
- Test: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add tests for helper decisions used by the runtime wiring:

- opening from menubar should target `RegularForeground`
- hide-to-tray should target `AccessoryBackground`
- overlay-only activity should keep `AccessoryBackground`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test surface_mode --manifest-path src-tauri/Cargo.toml`
Expected: FAIL because `lib.rs` still uses direct show/hide logic without surface-mode transitions.

- [ ] **Step 3: Implement runtime wiring**

Refactor `lib.rs` so that:

- `show_main_window` switches to `Regular` before showing/focusing
- hide-to-tray switches to `Accessory` after hiding, when no foreground surfaces remain
- initial menu bar preview setup applies the right initial surface mode

Keep current menu bar icon, open/quit items, and quit path.

- [ ] **Step 4: Run focused tests**

Run: `cargo test macos_system_surface surface_mode --manifest-path src-tauri/Cargo.toml`
Expected: PASS

### Task 3: Prevent Overlay From Promoting The App

**Files:**
- Modify: `src-tauri/src/providers/notification/overlay.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/macos_system_surface.rs`

- [ ] **Step 1: Write the failing test**

Add a pure test around the surface-policy inputs proving:

- hidden app + overlay visible => still `AccessoryBackground`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test overlay_visible_does_not_promote --manifest-path src-tauri/Cargo.toml`
Expected: FAIL until the policy explicitly models overlay as background-only.

- [ ] **Step 3: Implement minimal protection**

Ensure overlay display paths can explicitly re-apply / preserve `Accessory` mode when the app is hidden, instead of leaving the app in `Regular`.

- [ ] **Step 4: Run focused tests**

Run: `cargo test overlay_visible_does_not_promote --manifest-path src-tauri/Cargo.toml`
Expected: PASS

### Task 4: Verify The macOS Surface Behavior

**Files:**
- Modify: `src-tauri/src/lib.rs` or `src-tauri/src/services/macos_system_surface.rs` if verification reveals issues

- [ ] **Step 1: Run Rust tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS

- [ ] **Step 2: Run build verification**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: PASS

- [ ] **Step 3: Manually verify GUI behavior if session allows**

Run: `pnpm tauri dev`
Expected:

- visible main window => Dock icon visible
- hide main window => Dock icon disappears, menubar icon remains
- trigger overlay while hidden => Dock icon does not reappear
- open from menubar => Dock icon returns and window focuses
