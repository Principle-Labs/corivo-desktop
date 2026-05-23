# Migration gaps

This snapshot is an open-source cut of the upstream private monorepo. The cloud-capability trait split is in place, the agent and MCP sidecar sources are present, and the built-in connector catalog has been reduced to local CLI-style connectors. SaaS account integrations in the closed build now flow through the Corivo backend's Composio gateway behind `corivo-cloud`; the OSS build keeps that capability off by default.

## High priority

### 1. OSS integration strategy

The closed build's SaaS integrations are now backend-mediated through Composio. That is intentionally not enabled in the OSS binary because it depends on Corivo's hosted account pool and API routes.

Forks that want Gmail / Slack / Notion / Linear-style integrations should choose one path:

- Run their own backend gateway and implement the `ConnectorsService` trait.
- Keep integrations local-only and expose user-configured MCP servers / CLI tools.
- Build a separate OAuth proxy so desktop clients never ship provider secrets.

Do not reintroduce desktop-embedded OAuth secrets for public binaries. The Google sign-in flow in upstream has moved to backend-driven PKCE so the desktop no longer holds Google client secrets.

### 2. Build and sidecar packaging

`packages/agent/` and `packages/mcp/` are present, but end-to-end app startup still depends on staging the expected sidecar binaries under `apps/desktop/src-tauri/binaries/`. Verify the sidecar prep scripts on every supported platform before shipping binaries.

## Medium priority

### 3. Optional updater endpoint + signing

The OSS copy intentionally does not register Tauri's updater plugin. To ship signed updates, a fork needs to:

- Generate a fresh minisign keypair (`tauri signer generate`).
- Register `tauri_plugin_updater` from its private build hook.
- Put the public half in `tauri.conf.json#plugins.updater.pubkey` and add endpoint URLs.
- Add the corresponding `TAURI_SIGNING_PRIVATE_KEY` secret to release CI.
- Produce `latest.json` from `tauri build --bundles updater` and attach it to each release.

### 4. macOS signing identity

`tauri.conf.json#bundle.macOS.signingIdentity` is removed from the OSS copy. Local builds use ad-hoc signing; shipping a notarized `.app` requires adding a fork-owned `Developer ID Application` certificate and notarization workflow.

### 5. CI

No CI is included yet. The minimum useful set:

- `cargo check --no-default-features`
- `cargo test --lib`
- `pnpm --filter @corivo/desktop test`
- A secret/reference lint for `sk_live_`, provider `client_secret`, and accidental Corivo-hosted endpoint usage in OSS-only paths.

## Low priority / cleanup

### 6. `ai.corivo.desktop.community` bundle identifier

The OSS `tauri.conf.json` uses `ai.corivo.desktop.community` so it does not collide with the closed Corivo app. Forks redistributing under another brand should pick their own reverse-DNS identifier.

### 7. Legacy connector docs

`apps/desktop/docs/connector-framework-*` still describe the older per-SaaS connector package model. Treat those files as historical design notes until they are rewritten around the Composio / fork-owned-gateway split.

### 8. Privacy filter is wired but not yet classifying at capture

`apps/desktop/docs/privacy-filter-spec.md` lays out a "classify once at capture, enforce at egress" architecture; the OSS tree now ships:

- The `privacy_filter` service (`apps/desktop/src-tauri/src/services/privacy_filter/`) — model download, decode, redact, in-memory LRU cache.
- The egress enforce **Hook B** — `commands/exec_agent.rs::exec_agent_send` runs `primary_text` / `selection` through `enforce` before the `corivo_agent` sidecar sees them.
- The `frames.ax_text_pii_spans` schema column (v1500), the typed IPC + settings UI, and the six `commands/privacy::*` Tauri commands.

Still **not wired**: the capture-time classify Hook A — the call into `services::privacy_filter::classify` from the capture pipeline that would populate `frames.ax_text_pii_spans`. Until it lands, `spans` stays NULL on every frame, `enforce` gets an empty spans array, and the redact path is a no-op straight through. Users can flip the setting on without harm; nothing actually changes yet.

Hosting the q4f16 ONNX model is a separate fork concern: the OSS source has the download UI but no canonical URL. Forks shipping a release need to either bundle the model or point `services::privacy_filter::download` at their own CDN.

### 9. `persona_note_conflict.md` is a deferred spec, not dead code

`apps/desktop/src-tauri/prompts/persona_note_conflict.md` has no `include_str!` call site, but that is intentional — it is the system prompt for the per-paragraph LLM conflict judge described in memory-system-spec §5.1. The v0 implementation in `services/persona/conflict.rs` uses a substring-negation heuristic instead to save the per-paragraph LLM round-trip; when evaluation shows the heuristic is too crude, swap `conflict::annotate_conflicts` for an LLM call that uses this prompt and remove this item.

(`default_soul.md` was also in this slot historically. It was a stale seed for the pre-v1400 `Soul.md` user-edit surface, which the memory-system rewrite removed — personality now flows through the `<persistent_memory>` block in `services/exec_agent/local_context.rs`. The file has been deleted.)
