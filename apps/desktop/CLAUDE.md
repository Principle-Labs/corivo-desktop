# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working in `apps/desktop/`. For monorepo-level conventions (workspace layout, ts-rs pipeline, `@repo/*` vs `@corivo/*` packages), see [the root CLAUDE.md](../../CLAUDE.md).

> **Project status:** Corivo 尚未发布。优先把方案做对、做干净，少考虑向后兼容与历史包袱。

## What this app is

Corivo is a macOS desktop app built with Tauri 2 (Rust) + React 19. It captures the user's screen on a timer, extracts text via AX/OCR/per-app adapters, and lets the user **search, chat with, and pin** that history.

The current architecture is **"frames-as-truth"** (v3 spec, schema_version `1000`). The single source of truth is the `frames` table — one row per capture, holding `ax_text` + `ocr_text` + adapter payload + screenshot path. Everything else (chat, Quick Ask, embeddings) is a derived surface over `frames`.

The driving spec lives at [docs/corivo-architecture-v3-spec.md](docs/corivo-architecture-v3-spec.md). Read that before any non-trivial change.

> Older specs in `docs/` (event-task-project-spec, gum-refactor-spec, project-layer-spec, etc.) describe **architectures that have been ripped out**. They're kept for context but are not how the code works today.

## Development commands

Run from the repo root or from `apps/desktop/`:

```bash
# Frontend dev server only (Vite on port 1420)
pnpm --filter @corivo/desktop dev:vite

# Full Tauri app (Rust + Vite). Same as the root shortcut `pnpm app:dev`
pnpm --filter @corivo/desktop dev

# Build distributable .app / .dmg for the host arch (root: `pnpm app:build`)
pnpm --filter @corivo/desktop build

# Build distributable .app / .dmg for x86_64 (root: `pnpm app:build:x64`)
pnpm --filter @corivo/desktop build:x64

# Frontend tests (Vitest, environment: node, globals: true)
pnpm --filter @corivo/desktop test                # all
npx vitest run src/lib/tauri-capabilities.test.ts # single file (run from apps/desktop/)

# Rust tests (run from apps/desktop/src-tauri/)
cargo test                              # all
cargo test --lib                        # unit tests only
cargo test --test time_discipline       # one integration file
cargo test test_name -- --exact         # one test function

# ts-rs typegen — writes packages/shared-types/src/generated/.
# After running, also add the new file to packages/shared-types/src/index.ts.
pnpm --filter @corivo/desktop typegen
```

Vitest defaults to `environment: "node"` (no DOM). Tests that need DOM must opt in per file with `// @vitest-environment jsdom` or a config override.

## Three windows, one Vite build

[vite.config.ts](vite.config.ts) declares three Rollup inputs:

| HTML entry | React root | Window label | Role |
|------------|------------|--------------|------|
| `index.html` | `src/main.tsx` | `main` | Main app — router + sidebar + pages (`/timeline`, `/ask`, `/settings`, `/onboarding/*`). |
| `notification-overlay.html` | `src/overlay/main.tsx` | `notification-overlay` | NSPanel-hosted notification overlay (independent React root, no router). |
| `quick-ask.html` | `src/overlay-quick-ask/main.tsx` | `quick-ask` | Global-hotkey panel (always-on-top, transparent, decoration-less). Created at boot via [tauri.conf.json](src-tauri/tauri.conf.json) and toggled by `on_quick_ask_hotkey` in [src-tauri/src/lib.rs](src-tauri/src/lib.rs). |

When adding a fourth window, both `vite.config.ts` (Rollup input) and `tauri.conf.json` (`app.windows[]`) need entries.

## Rust backend (`src-tauri/src/`)

| Layer | Path | Role |
|-------|------|------|
| **commands/** | Tauri `#[command]` handlers | Thin entry points exposed via `invoke()`. Files split by surface: `capture`, `chat`, `config`, `frames`, `llm`, `notification`, `onboarding`, `quick_ask`, `settings`. The full `invoke_handler!` list lives in `lib.rs::run()`. |
| **services/** | Long-running services | One per responsibility (see below). All wired in `lib.rs::run()` and stored on `AppState`. |
| **providers/** | External integrations | `llm/` (Codex / Gemini / mock / unavailable + pricing); `notification/` (overlay panel via NSPanel + screen metrics). |
| **db/** | SQLite via rusqlite + r2d2 | `db/schema.sql` is the single source of truth. `db/migrations.rs` does **purge-and-apply** on version mismatch. Repos in `db/repos/` (frames, embeddings, chat). |
| **domain/** | Domain types | `config.rs` (persisted via `tauri-plugin-store`), `ipc_error.rs` (tagged `TauriError` shared with the frontend), plus DTOs for chat / frame / embedding / focus_context / snapshot_envelope. |

### The capture → frame pipeline

```
capture_pipeline (timer | focus_change | quick_ask trigger)
    → screen_capture → dedup → focus_watcher → foreground (AX probe)
    → extractor::dispatcher (per-app adapter or generic AX or OCR fallback)
    → snapshot_consumer::LocalConsumer
    → frames table (one row, with ax_text + ocr_text + adapter_payload + screenshot)
    → frames_fts (jieba-tokenized search_tokens column)

Background sweeps:
- frame_indexer    → embedding generation (currently DISABLED at boot in lib.rs;
                     FTS-only recall via hybrid_search.rs is the demo mode)
- retention        → hourly sweep, drops frames + screenshots past the 90-day cutoff
```

### `/ask` and Quick Ask (the recall layer)

Both UIs share the same orchestrator:

- `services::recall::orchestrator::Orchestrator` runs the LLM ↔ tool-call loop. System prompts are `recall_orchestrator.md` for `/ask` and `quick_ask_orchestrator.md` for Quick Ask. Loop budget is `MAX_TOOL_ROUNDS`.
- Tools are defined in `tool_defs.rs` and dispatched by `tools.rs`. They read frames + embeddings + capture screenshots via `ToolDeps`.
- `hybrid_search.rs` is the actual retrieval — FTS5 over the jieba-tokenized `search_tokens` column today; vector search wires up automatically once `frame_indexer` is re-enabled.
- `stream.rs` wraps a Tauri `Channel<T>` so `chat_send_message` streams `TextDelta` / `tool-call` / `tool-result` / `finish` events to the frontend incrementally.
- Conversation history persists in `chat_threads` + `chat_messages`. `chat_messages.tool_calls` and `cited_frame_ids` are JSON columns; the frontend reconstructs the Vercel AI SDK Data Stream Protocol view from them.

### Quick Ask threading note

The global hotkey handler in `lib.rs` captures the user's foreground app **before** showing the Quick Ask window — otherwise the AX probe would read Corivo itself once `window.show()` activates the app. Every `WebviewWindow::{show, hide, set_focus, is_visible}` call goes through `run_on_main_thread` (helper: `run_on_main`) because AppKit window ops on a tokio worker leak `NSException` through FFI and abort the runtime. Same constraint that `providers/notification/panel.rs` guards against.

### Other key services

- **`capture_store`** — filesystem screenshot storage under `$APPDATA/captures/`.
- **`config_service`** — wraps `tauri-plugin-store`. **All secrets** (Anthropic API key, Corivo session token) live here in plaintext under `Config.exec_agent.anthropic_api_key` / `Config.corivo_session.session_token`. No OS keychain.
- **`llm_service`** — provider router; reads keys from `config_service`, model selection from `config_service`.
- **`notification_service`** — NSPanel-backed overlay (currently in low-traffic mode).
- **`hotkey`** — bookkeeping only; the actual `global-shortcut` handler is registered in `lib.rs::run()` after `AppState` is mounted (registering earlier would race with an empty state).
- **`exclusion`** — per-app capture skip list (bundle ids), extra entries persisted in `Config`.
- **`extractor`** — per-app adapter registry. Today ships `GenericAxAdapter` only; per-app adapters land in Batch 12.
- **`macos_system_surface`** — applies/syncs the macOS `NSApplication.activationPolicy` (regular ↔ accessory) so hiding the main window correctly removes the dock icon.
- **`tokenize`** — jieba dictionary; warmed up at boot via `spawn_blocking` so the first FTS query doesn't pay 200–500 ms init.

## Sidecars and macOS TCC (CO-31)

Three sidecar processes ship inside `Corivo.app`. They take two different delivery shapes:

| Sidecar | Source | Delivery shape | Path inside `Corivo.app` |
|---|---|---|---|
| `corivo-mcp` | `packages/mcp` (Rust) | Flat Mach-O via `bundle.externalBin` | `Contents/MacOS/corivo-mcp` |
| `corivo-agent` | `packages/agent` (Bun --compile) | Flat Mach-O via `bundle.externalBin` | `Contents/MacOS/corivo-agent` |
| `CorivoCaptureHelper` | `packages/desktop-helpers/macos` (Swift) | **`.app` bundle** via `bundle.resources` | `Contents/Resources/CorivoCaptureHelper.app/Contents/MacOS/CorivoCaptureHelper` |

The capture helper is an .app bundle, not a flat binary, for two reasons:

1. **CO-31 fix.** The helper links AppKit / AVFoundation / ScreenCaptureKit. Without an `Info.plist` declaring `LSUIElement = YES`, Launch Services treats it as a regular GUI app and bounces its icon in the Dock forever waiting for `NSApplicationDidFinishLaunching` (which never comes). The .app form gives us that plist key.
2. **TCC responsible-process attribution.** macOS routes permission requests up the responsible-process chain back to the parent. For that to work, the helper needs:
   - To be embedded inside the parent `.app` (i.e. under `Contents/Resources/`)
   - To be signed with the same Team ID + Hardened Runtime as the parent
   - Its own `CFBundleIdentifier` (`ai.corivo.desktop.capture-helper`)
   - **No `NS*UsageDescription` of its own** — those all live in the parent's `Info.plist`. If the helper declares its own, tccd treats it as a separate TCC subject and the user sees both "Corivo" and "Corivo Capture Helper" in System Settings → Privacy.

Files in play:

- [src-tauri/Info.plist](src-tauri/Info.plist) — partial Info.plist merged into the Tauri-generated one. **All `NS*UsageDescription` keys live here.** Do NOT add usage descriptions to the helper.
- [src-tauri/Parent.entitlements](src-tauri/Parent.entitlements) — Hardened Runtime entitlements for `Corivo.app`. Includes the TCC categories used by the helper (microphone / audio-input / apple-events) so attribution works.
- [packages/desktop-helpers/macos/HelperInfo.plist](../../packages/desktop-helpers/macos/HelperInfo.plist) — minimal 4-key `Info.plist` for the helper bundle. Read by `build.sh`.
- [packages/desktop-helpers/macos/Helper.entitlements](../../packages/desktop-helpers/macos/Helper.entitlements) — minimal entitlements for the helper.

`lib.rs::run()` calls `accessibility_sys::AXIsProcessTrusted()` once at boot (`#[cfg(target_os = "macos")]`) before spawning the helper. This no-prompt probe is what registers `ai.corivo.desktop` in tccd's known-clients table for Accessibility. Without it, AX is the one TCC category that may still show "Corivo Capture Helper" as a separate entry — see the Littlebird responsible-process notes if you need to reproduce the call path.

### Dev-mode TCC caveat

In `tauri:dev`, the parent process is `target/debug/Corivo` (a bare Mach-O, not a `.app` bundle), so the responsible-process chain is broken: TCC will record `Corivo` and `CorivoCaptureHelper` as **separate** entries on your dev machine. This is expected and only affects developers — the production build (`tauri:build`) ships a real `.app` and merges cleanly.

If you want dev parity, build a release once with `pnpm --filter @corivo/desktop tauri:build`, run it from `target/release/bundle/macos/Corivo.app`, and grant permissions there — that's the layout users actually run.

## Sidecars on Windows

Windows ships every sidecar — including the capture helper — as a **flat `.exe` via `bundle.externalBin`**. No `.app` bundle equivalent is needed (no TCC, no Dock icon problem).

### Layered config (why there's a `tauri.windows.conf.json`)

Tauri 2's `bundle.externalBin` schema has no per-platform field, but Tauri auto-merges `tauri.<platform>.conf.json` files via JSON Merge Patch (RFC 7396). So:

- [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json) — base, lists `corivo-mcp` + `corivo-agent` only.
- [src-tauri/tauri.windows.conf.json](src-tauri/tauri.windows.conf.json) — Windows-only override; replaces `bundle.externalBin` with the base list **plus** `binaries/corivo-capture-helper`.

RFC 7396 replaces arrays whole rather than appending, so the Windows override has to re-declare the base entries. **When adding a new sidecar to the base list, also update the Windows override.**

Why not put the capture helper in the base list at all: Tauri's bundling validates that every `externalBin` entry exists with a `-<target-triple>` suffix at build time. On macOS the capture helper is delivered as a `.app` bundle in `bundle.resources`, not a flat binary — there's no `corivo-capture-helper-aarch64-apple-darwin` to point at. Listing the helper in the base array would force the macOS build to ship a dummy Mach-O stub solely to pass validation.

### Runtime path resolution

[src-tauri/src/lib.rs](src-tauri/src/lib.rs) `locate_capture_helper_binary` handles both:

- **Production**: `corivo-capture-helper.exe` next to `Corivo.exe` (where Tauri's bundling drops `externalBin` entries on Windows).
- **Dev**: `src-tauri/binaries/corivo-capture-helper-<triple>.exe` (where `packages/desktop-helpers/windows/build.ps1` stages the freshly-built binary).

[src-tauri/scripts/prep-sidecar.mjs](src-tauri/scripts/prep-sidecar.mjs) seeds an empty placeholder at the dev path on Windows so a fresh checkout's `cargo check` / `tauri dev` validation passes before the real helper has been built. To produce the real helper, run `pnpm --filter @corivo/desktop-helpers run build` (or `build:release`) — that invokes `windows/build.ps1` and overwrites the placeholder.

## Frontend (`src/`)

| Path | Role |
|------|------|
| `app/` | Boot sequence (`app-boot.tsx` handles updater + onboarding redirect), router, layout. |
| `routes/` | TanStack Router flat-file route definitions (`timeline.tsx`, `ask.tsx`, `settings.tsx`, `onboarding.*.tsx`). `/` redirects to `/timeline`. |
| `pages/timeline/` | Frame timeline UI — list / filter / open frame detail with screenshot. |
| `pages/ask/` | `/ask` chat UI; consumes the streaming channel from `chat_send_message`. |
| `pages/settings/` | API keys / capture / exclusions / data / hotkey / about. |
| `pages/onboarding/` | Welcome → permissions → API keys → done. |
| `overlay/` | Notification overlay React root (`notification-overlay.html`). |
| `overlay-quick-ask/` | Quick Ask React root (`quick-ask.html`). Listens to `quick-ask:opened` / `quick-ask:error` window-scoped events emitted from `lib.rs`. |
| `lib/tauri.ts` | Typed wrappers around every `invoke()` call — **single source of truth** for frontend↔backend IPC. |
| `lib/types.ts` | TypeScript mirror of cross-boundary Rust types. Re-exports auto-generated types from `@corivo/shared-types` (today: `TauriError`); the rest is still hand-written and migrates one struct at a time as Rust gets `#[derive(TS)]`. |
| `hooks/use-frames.ts` `use-chat.ts` `use-capture.ts` `use-config.ts` | React Query hooks. |
| `stores/` | Zustand stores (`updater-store`). |
| `components/layout/` | App-local layout components. **All shadcn primitives live in `@repo/ui`** — import them as `@repo/ui/components/<name>`. Don't recreate them here. |

Path alias `@/` → `apps/desktop/src/` (configured in both `tsconfig.json` and `vite.config.ts`). Cross-app imports must NOT go through `@/` — use `@repo/ui`, `@corivo/shared-types`, etc.

## Config system

- **Backend canonical struct:** `domain::config::Config` (persisted to JSON via `tauri-plugin-store`).
- **Frontend mirror:** `lib/types.ts::Config`. Read/written via `getConfig` / `setConfig` IPC wrappers.
- **API keys live IN `Config`** (plaintext): `exec_agent.anthropic_api_key` for the user's `sk-ant-…`, `corivo_session.session_token` for the closed-beta bearer token. The `get_api_key_status` / `save_anthropic_api_key` / `delete_anthropic_api_key` commands are a thin convenience layer over `setConfig`.
- Some `Config` fields may be dead remnants of removed pipelines (push / drift evaluation / GUM era). Kept to avoid breaking existing `config.json` files. If you add a new field, update both Rust and TS sides and bump the migration if defaults change.

## Time discipline (`db::time`)

- `DbInstant` is the **only** type that crosses the DB timestamp boundary. It implements `ToSql` / `FromSql` with canonical RFC3339-with-millis write (`strftime('%Y-%m-%dT%H:%M:%fZ','now')`) and a tolerant read parser.
- In-memory `DateTime<Utc>` for business logic goes through `now_utc()`.
- Both route through a global `Clock` (`SystemClock` by default, `FakeClock` in tests via `set_clock`).
- `datetime('now')` is **banned** in application SQL. `tests/time_discipline.rs` grep-scans every `.rs` / `.sql` under `src/` + `tests/` and fails CI if the literal reappears outside a small allowlist.

## Database

SQLite at `$APPDATA/corivo.sqlite`, `schema_version = 1000`.

**Live tables** (per `db/schema.sql`):
- `frames` + `frames_fts` (FTS5 over the jieba-tokenized `search_tokens` column) — capture truth.
- `frame_embeddings` — vector index, raw little-endian f32 bytes; cosine similarity in Rust (full-table scan; fine at <100k frames).
- `chat_threads` + `chat_messages` — `/ask` + Quick Ask history (also serves as the user's "save this conversation" surface via `quick_ask_pin_thread`).

**Migration model:** `db/migrations.rs::apply_migrations` compares `schema_version` against `TARGET_SCHEMA_VERSION` and, on mismatch, **drops every legacy table it knows about then runs `schema.sql`**. No ladder. Pre-release project; historical rows are intentionally discardable. When `schema.sql` changes, bump `TARGET_SCHEMA_VERSION` AND keep the legacy-drop list in `migrations.rs` complete enough that a long-time dev's `.sqlite` survives.

## Prompts

Every prompt lives under `src-tauri/prompts/`. Each call site `include_str!`s the `.md` template, `replace()`s `{{slot}}` placeholders, sends the filled string as `LlmRequest::json` with a JSON schema, and deserializes the model's reply into a `response::*` struct with serde.

Current prompts:
- `frame_summarize.md` — multi-frame summarization (used by recall tooling).
- `recall_orchestrator.md` — `/ask` system prompt (drives the tool-use loop).
- `quick_ask_orchestrator.md` — Quick Ask variant of the recall system prompt.

## Conventions

- **Path alias:** `@/` → `./src/`. Cross-app sharing uses `@repo/ui`, `@corivo/shared-types`, etc.
- **Error type:** `error::CorivoError` (`thiserror`). Tauri commands return `Result<T, TauriError>` (the tagged union in `domain::ipc_error`); callers narrow with `fromInvokeError` on the JS side.
- **UI strings** are in 中文; config / code identifiers stay in English.
- The app uses a custom titlebar (`TitleBarStyle::Overlay` + `hiddenTitle`) — `TitleBar` component handles the drag region.
- Custom tray icon + menu (`Open Corivo` / `Quit Corivo`) wired in `lib.rs::setup_menu_bar_preview`. Closing the main window hides to tray when `Config.app.minimize_to_tray` is on.
- **When adding a new service:** put it in `services/<name>/{mod.rs, ...}`, spawn it from `lib.rs::run()` setup, pass deps explicitly. Follow `recall::Orchestrator` or `retention::RetentionTask` as the template.
- **When adding a new Tauri command:** add the handler to the relevant `commands/*.rs`, wire it in `lib.rs::invoke_handler!`, add a typed wrapper in [src/lib/tauri.ts](src/lib/tauri.ts), and — if it returns a Rust struct — add `#[derive(TS)]` + `export_to` so the type is auto-generated into `@corivo/shared-types` (then re-export it from [packages/shared-types/src/index.ts](../../packages/shared-types/src/index.ts)).
- **When adding a window-scoped event** (Tauri `emit_to`): the receiving React root listens via `getCurrentWebviewWindow().listen(...)`. See `quick-ask:opened` for the pattern.
