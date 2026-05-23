# Menu Bar Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a minimal macOS menu bar preview for Corivo with a template tray icon, open/quit actions, and working close-to-tray behavior.

**Architecture:** Keep the implementation local to `src-tauri/src/lib.rs`, using Tauri's tray API for runtime wiring and a few pure helper functions for policy decisions. Reuse the existing `minimize_to_tray` config and the existing `menubar.png` asset instead of introducing a larger lifecycle refactor.

**Tech Stack:** Rust, Tauri 2, tauri tray/menu APIs, cargo test

---

### Task 1: Lock In Close-To-Tray Policy With Tests

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add unit tests for a pure helper that decides the main window close action:

- `minimize_to_tray = true` => hide window
- `minimize_to_tray = false` => allow shutdown

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test close_request --manifest-path src-tauri/Cargo.toml`
Expected: FAIL because the helper does not exist yet.

- [ ] **Step 3: Implement the helper**

Add a small enum and helper in `src-tauri/src/lib.rs` that resolve close behavior from config.

- [ ] **Step 4: Re-run the test**

Run: `cargo test close_request --manifest-path src-tauri/Cargo.toml`
Expected: PASS

### Task 2: Lock In Tray Actions With Tests

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add unit tests for tray menu id parsing:

- `tray-open` => open main window
- `tray-quit` => quit app
- unknown ids => ignored

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test tray_menu --manifest-path src-tauri/Cargo.toml`
Expected: FAIL because the parser does not exist yet.

- [ ] **Step 3: Implement the helper**

Add tray item id constants and a pure parser helper in `src-tauri/src/lib.rs`.

- [ ] **Step 4: Re-run the test**

Run: `cargo test tray_menu --manifest-path src-tauri/Cargo.toml`
Expected: PASS

### Task 3: Wire The Runtime Tray Preview

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Implement the tray setup**

In `setup`, create:

- a tray menu with `Open Corivo` and `Quit Corivo`
- a tray icon from `src-tauri/icons/menubar.png`
- `icon_as_template(true)` on macOS
- a click handler that restores the main window

- [ ] **Step 2: Implement close interception**

Update `on_window_event` so main-window close requests hide the window when `minimize_to_tray` is enabled, while keeping existing shutdown logic for real exits.

- [ ] **Step 3: Add quit handling**

Route the tray `Quit Corivo` action through a helper that shuts down the capture store and DB once, then exits the app.

### Task 4: Verify The Preview End-To-End

**Files:**
- Modify: `src-tauri/src/lib.rs` if verification reveals issues

- [ ] **Step 1: Run targeted tests**

Run: `cargo test close_request tray_menu --manifest-path src-tauri/Cargo.toml`
Expected: PASS

- [ ] **Step 2: Run the full Rust test/build verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: PASS

- [ ] **Step 3: Launch for manual preview if the local GUI session allows it**

Run: `pnpm tauri dev`
Expected: Corivo starts with a menu bar icon visible in the macOS status bar.
