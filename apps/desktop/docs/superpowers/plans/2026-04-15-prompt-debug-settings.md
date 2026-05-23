# Prompt Debug Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a debug-only settings subpage where users can override the screenshot-summary prompt and memory-push judgment prompt, while preserving built-in defaults and an instant reset path.

**Architecture:** Extend the persisted config with a small `prompt_debug` section, keep built-in default prompts in the pipeline, and resolve the effective prompt at runtime via `override ?? default`. Surface those two override fields through a dedicated `Prompt 调试` settings section in the frontend.

**Tech Stack:** Tauri, Rust, React, TanStack Query, TanStack Router, Vitest

---

### Task 1: Extend Config Types With Prompt-Debug Overrides

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src/lib/types.ts`
- Test: `src-tauri/tests/config_services.rs`

- [ ] **Step 1: Write the failing config test**

Add a Rust test that seeds config without `prompt_debug` and asserts:

- deserialization still succeeds
- `prompt_debug.summary_override == None`
- `prompt_debug.push_judgment_override == None`

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test config_services --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because `PromptDebugConfig` does not exist yet.

- [ ] **Step 3: Add the new config section**

In `src-tauri/src/domain/config.rs`:

- add `PromptDebugConfig`
- add `prompt_debug: PromptDebugConfig` to `Config`
- derive `Serialize`, `Deserialize`, `PartialEq`, `Eq`
- use `#[serde(default)]`
- default both override fields to `None`

In `src/lib/types.ts`:

- add matching `PromptDebugConfig`
- extend `Config`

- [ ] **Step 4: Re-run the config test**

Run: `cargo test --test config_services --manifest-path src-tauri/Cargo.toml`

Expected: PASS

### Task 2: Normalize Empty Overrides to Default Semantics

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src-tauri/src/services/config_service.rs`
- Test: `src-tauri/tests/config_services.rs`

- [ ] **Step 1: Write the failing normalization test**

Add a test that updates config with:

- `summary_override = Some("")`
- `push_judgment_override = Some("   ")`

and asserts both are persisted as `None`.

- [ ] **Step 2: Run the normalization test**

Run: `cargo test --test config_services prompt_debug --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because whitespace-only overrides are not normalized yet.

- [ ] **Step 3: Implement normalization**

Add a small helper in `Config::normalize()` or a dedicated prompt-debug normalizer that:

- trims override strings
- converts empty/whitespace-only strings to `None`
- preserves non-empty text verbatim

- [ ] **Step 4: Re-run the normalization test**

Run: `cargo test --test config_services prompt_debug --manifest-path src-tauri/Cargo.toml`

Expected: PASS

### Task 3: Make MVP Pipeline Resolve Effective Prompts From Config

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing pipeline tests**

Add tests covering two cases:

1. no overrides configured
   - summary path uses built-in `SUMMARY_PROMPT`
   - judgment path uses built-in `PUSH_JUDGMENT_PROMPT`
2. overrides configured
   - summary path uses `prompt_debug.summary_override`
   - judgment path uses `prompt_debug.push_judgment_override`

Reuse the existing fake LLM prompt capture in `src-tauri/tests/mvp_pipeline.rs`.

- [ ] **Step 2: Run the pipeline tests**

Run: `cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`

Expected: FAIL because prompts are still hardcoded constants at call sites.

- [ ] **Step 3: Inject config access into the pipeline**

Refactor `src-tauri/src/services/mvp_pipeline.rs` so the service can resolve prompts via helper methods such as:

```rust
fn summary_prompt(&self) -> String
fn push_judgment_prompt_template(&self) -> String
```

Rules:

- read current config through a small dependency
- use override when present
- otherwise return built-in default

Do not remove the built-in constants.

- [ ] **Step 4: Re-run the pipeline tests**

Run: `cargo test --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`

Expected: PASS

### Task 4: Add a Dedicated Settings Section for Prompt Debugging

**Files:**
- Modify: `src/pages/settings/settings-page.tsx`
- Create: `src/pages/settings/sections/prompt-debug-section.tsx`
- Test: `src/pages/settings/sections/prompt-debug-section.test.tsx`

- [ ] **Step 1: Write the failing component test**

Add a test that renders the settings page or the new section and verifies:

- the `Prompt 调试` section appears in navigation
- both textareas render
- existing config values populate the inputs

- [ ] **Step 2: Run the frontend test**

Run: `pnpm test -- prompt-debug-section`

Expected: FAIL because the new section does not exist yet.

- [ ] **Step 3: Implement the section UI**

In `src/pages/settings/settings-page.tsx`:

- add a `prompt-debug` section id and label
- render the new section component

In `src/pages/settings/sections/prompt-debug-section.tsx`:

- use `useConfig()`
- maintain local draft state for both inputs
- show `保存`, `恢复默认`, `复制当前生效版本`
- show source badges or text: `默认值` / `用户覆盖`
- add concise descriptions of what each prompt affects

- [ ] **Step 4: Re-run the component test**

Run: `pnpm test -- prompt-debug-section`

Expected: PASS

### Task 5: Wire Save/Reset Behavior to `set_config`

**Files:**
- Modify: `src/pages/settings/sections/prompt-debug-section.tsx`
- Modify: `src/lib/config-tauri.test.ts`
- Test: `src/pages/settings/sections/prompt-debug-section.test.tsx`

- [ ] **Step 1: Write the failing save/reset tests**

Cover:

- clicking `保存` sends updated `prompt_debug` payload
- clicking `恢复默认` stores `None` semantics rather than an empty string
- buttons disable appropriately while saving

- [ ] **Step 2: Run the tests**

Run: `pnpm test -- config-tauri prompt-debug-section`

Expected: FAIL because the payload does not include the new config fields yet.

- [ ] **Step 3: Implement save/reset logic**

Use the existing `useConfig().update()` flow:

- `保存` writes current text
- `恢复默认` writes `null` / absence semantics via config shape
- preserve other config branches untouched

Update `src/lib/config-tauri.test.ts` to include `prompt_debug` in the expected config object.

- [ ] **Step 4: Re-run the tests**

Run: `pnpm test -- config-tauri prompt-debug-section`

Expected: PASS

### Task 6: Keep Prompt Testing and Runtime Override Semantics Clear

**Files:**
- Modify: `src/pages/settings/sections/api-keys-section.tsx`
- Test: `src/pages/settings/sections/api-keys-section.test.tsx` or existing related tests if present

- [ ] **Step 1: Add a small failing UI test or assertion**

Cover that the existing prompt test card clearly indicates it is for temporary experiments and not the runtime pipeline override.

- [ ] **Step 2: Run the related test**

Run: `pnpm test -- api-keys-section`

Expected: FAIL if no such copy exists yet.

- [ ] **Step 3: Update the copy**

Adjust the card description in `src/pages/settings/sections/api-keys-section.tsx` so it states:

- this card tests a temporary prompt against uploaded images
- formal runtime prompt overrides live in `Prompt 调试`

- [ ] **Step 4: Re-run the related test**

Run: `pnpm test -- api-keys-section`

Expected: PASS

### Task 7: Final Verification

**Files:**
- Verify only

- [ ] **Step 1: Run backend verification**

Run: `cargo test --test config_services --test mvp_pipeline --manifest-path src-tauri/Cargo.toml`

Expected: PASS

- [ ] **Step 2: Run frontend verification**

Run: `pnpm test -- config-tauri prompt-debug-section api-keys-section`

Expected: PASS

- [ ] **Step 3: Manual smoke check**

Run the app and verify:

1. Open `设置 > Prompt 调试`
2. Edit `截图总结 Prompt`, save, restart app, confirm the value persists
3. Trigger a summary flow and confirm the decision log or captured prompt reflects the override
4. Click `恢复默认` and confirm the app falls back to the built-in prompt

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/domain/config.rs src-tauri/src/services/config_service.rs src-tauri/src/services/mvp_pipeline.rs src-tauri/src/commands/config.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/config-tauri.test.ts src/pages/settings/settings-page.tsx src/pages/settings/sections/prompt-debug-section.tsx src/pages/settings/sections/api-keys-section.tsx src-tauri/tests/config_services.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: add prompt debug settings"
```
