# Corivo Desktop (Community Edition)

**English** · [中文](./README.zh-CN.md)

The open-source distribution of the Corivo desktop client — a macOS Tauri + React app that captures the screen on a timer, extracts text via AX / OCR / per-app adapters, and lets you search, chat with, and pin that history locally.

This repository contains the parts of Corivo that work without any hosted backend. The closed Corivo build (Google sign-in, hosted model gateway, Stripe top-ups, managed integrations, Sentry telemetry, signed updater policy) is **not** included; the trait-based capability layer in `apps/desktop/src-tauri/src/services/cloud/` makes those swappable, so forks can plug their own implementations in.

---

## Status

**Pre-release.** The capability split is recent and the `@corivo/agent` / `@corivo/mcp` sidecar sources are now included. The closed build's SaaS integrations have moved behind a backend-mediated connectors gateway that is disabled in the OSS build. Today you can build and iterate on the local desktop experience; hosted account, billing, managed model directory, managed integrations, telemetry, and signed updater policy are intentionally absent.

---

## What's in this tree

```
apps/
  desktop/              @corivo/desktop       — Tauri 2 + React 19 macOS app
packages/
  ui/                   @repo/ui              — shadcn/ui primitives + cn()
  desktop-helpers/      @corivo/desktop-helpers — cross-platform screen helper (Swift on macOS, C++ on Windows)
  agent/                @corivo/agent           — Bun-compiled chat / Quick Ask sidecar
  mcp/                  @corivo/mcp             — Rust MCP bridge sidecar
  shared-types/         @corivo/shared-types  — ts-rs generated Rust→TS types
  tailwind-config/      @repo/tailwind-config — Tailwind v4 preset
  tsconfig/             @repo/tsconfig        — TS configs
```

What's intentionally **not** here:

- `apps/api/` and `apps/web/` — Corivo's hosted backend and marketing site.
- `apps/desktop/src-tauri/src/services/cloud/corivo/` — closed-source cloud trait impls (Google sign-in, Stripe checkout, managed model directory, managed connectors, Sentry telemetry, corivo-policy updater).
- `apps/desktop/src-tauri/prompts/corivo/` — product-tuned prompts for the persona-distill and session-memory-learning background agents. The `prompts/*.md` files in this repo are simplified open-source defaults.
- Corivo's hosted connectors gateway and account pool. The OSS build keeps the connectors capability off by default; forks can implement their own backend gateway, OAuth proxy, or local-only MCP / CLI integration story.

---

## Architecture (the part you can read today)

The cloud-capability layer lives at:

- `apps/desktop/src-tauri/src/services/cloud/` — trait definitions (`AuthService`, `BillingService`, `ModelsService`, `ConnectorsService`, `TelemetryService`, `UpdaterPolicyService`) plus the all-noop default bundle.
- `apps/desktop/src-tauri/src/commands/cloud.rs` — single `get_capabilities` Tauri command that the frontend probes at boot.
- `apps/desktop/src/hooks/use-capabilities.ts` — React Query hook that caches the answer for the session.

Every cloud-coupled UI surface in `apps/desktop/src/` reads `useCapabilities()` and either renders or short-circuits:

- `/login` route navigation in `app/app-boot.tsx`
- `BillingDialog` in `app/layout.tsx`
- Settings → Integrations tab in `components/settings/settings-dialog.tsx`
- Sidebar account dropdown + balance chip in `components/layout/user-card.tsx`

For a fork that wants to wire a cloud backend, the pattern is:

1. Add a sibling module `apps/desktop/src-tauri/src/services/cloud/yourcloud/` with one impl per trait.
2. After building `CloudServices::noop()` in `lib.rs::run()`, mutate fields with your impls before wrapping in `Arc::new`.
3. Each impl's `is_available()` returning `true` lights up the matching UI.

---

## Development

Prerequisites: pnpm 11+, Rust stable + Tauri's platform deps (Xcode CLT on macOS, etc.).

```sh
pnpm install
pnpm app:dev               # full Tauri app (Rust + Vite, port 1420)
pnpm --filter @corivo/desktop dev:vite   # frontend only
pnpm --filter @corivo/desktop test       # vitest
```

**Note:** the chat / Quick Ask paths expect `corivo-agent` and `corivo-mcp` sidecars at `apps/desktop/src-tauri/binaries/`, plus a configured BYOK model/API key in Settings. The sidecars are built from `packages/agent/` and `packages/mcp/`; Bun is required for the agent compiler.

---

## License

Apache 2.0. See [LICENSE](./LICENSE).

Forks shipping a binary must change `tauri.conf.json#identifier` and `productName` so end-users can install the community build alongside (or instead of) any official Corivo build.

---

## Contributing

Pull requests welcome. Before sending one for non-trivial changes, please open an issue describing the direction.
