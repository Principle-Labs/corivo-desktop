// One-shot OAuth bootstrap for an `mcpServer`-shape connector.
//
// Called by the Rust host when the user clicks 安装 on an MCP-backed
// connector card. The sidecar binary is invoked as:
//
//   corivo-agent --bootstrap-mcp-oauth <input-file-path>
//
// where the input file contains a single `McpServerSpec`. We build a
// mcporter runtime with just that server, call `listTools` to trigger
// the OAuth flow (mcporter opens the system browser, listens on a
// loopback port, exchanges the code for a token, persists to the
// spec's `token_cache_dir`), then close and exit.
//
// Why a separate subcommand instead of letting the normal agent boot
// do this lazily on first tool call:
//   - UX: clicking 安装 should produce the browser tab immediately,
//     not silently when the model first reaches for a Linear tool.
//   - Determinism: bootstrap can succeed/fail synchronously; the host
//     waits on this child's exit code instead of polling agent events.
//   - Caching: the next normal `corivo-agent` invocation hits the
//     same `token_cache_dir` and reconnects without re-prompting.

import crypto from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { createRuntime } from "mcporter";

import { log } from "../log.js";
import type { McpServerSpec } from "../types.js";

interface BootstrapInput {
  server: McpServerSpec;
  /** Optional client-side OAuth code timeout (ms). Defaults to 5 minutes. */
  timeout_ms?: number;
}

interface ClearInput {
  server: McpServerSpec;
}

const DEFAULT_OAUTH_TIMEOUT_MS = 5 * 60 * 1000;

/**
 * Read the bootstrap input file, run mcporter against the single
 * server until OAuth completes, then exit. Returns the process exit
 * code so `main.ts` can `process.exit(code)` from one place.
 */
export async function runBootstrapMcpOauth(inputPath: string): Promise<number> {
  let parsed: BootstrapInput;
  try {
    const raw = await Bun.file(inputPath).text();
    parsed = JSON.parse(raw.trim()) as BootstrapInput;
  } catch (err) {
    log.error("mcp.bootstrap.parse_failed", {
      input_path: inputPath,
      error: (err as Error).message,
    });
    return 1;
  }

  const spec = parsed.server;
  const timeoutMs = parsed.timeout_ms ?? DEFAULT_OAUTH_TIMEOUT_MS;

  log.info("mcp.bootstrap.start", {
    server: spec.name,
    transport: spec.transport,
    token_cache_dir: spec.token_cache_dir ?? "(default)",
    timeout_ms: timeoutMs,
  });

  let runtime;
  try {
    runtime = await createRuntime({
      servers: [toServerDefinition(spec)],
      oauthTimeoutMs: timeoutMs,
    });
  } catch (err) {
    log.error("mcp.bootstrap.runtime_create_failed", {
      server: spec.name,
      error: (err as Error).message,
    });
    return 1;
  }

  try {
    // `listTools` is the cheapest path that forces a real connect (and
    // therefore an OAuth round-trip if no cached token). We don't care
    // about the tools — we throw away the list. The side effect is the
    // populated token cache directory.
    await runtime.listTools(spec.name, { includeSchema: false });
    log.info("mcp.bootstrap.ok", { server: spec.name });
    return 0;
  } catch (err) {
    log.error("mcp.bootstrap.list_tools_failed", {
      server: spec.name,
      error: (err as Error).message,
    });
    return 1;
  } finally {
    await runtime.close().catch((err) => {
      log.warn("mcp.bootstrap.close_error", {
        server: spec.name,
        error: (err as Error).message,
      });
    });
  }
}

/**
 * One-shot inverse of {@link runBootstrapMcpOauth}: revoke every
 * OAuth-related cache for the given MCP server. Called from the Rust
 * host when the user clicks 断开 on an mcpServer-shape connector.
 *
 * Calls mcporter's own `clearOAuthCaches` so both stores get wiped:
 *   - `tokenCacheDir` (Corivo-controlled per-server directory)
 *   - vault entry at `~/.mcporter/credentials.json` (shared file
 *     mcporter writes alongside the dir; not opt-out-able in 0.10.x)
 *
 * Wiping only one of the two — what we used to do with `rm -rf` on
 * the directory — left the vault behind, so the next install hit
 * the cached DCR client + token and "reconnected" in 2-3 seconds
 * without any real OAuth round-trip. That was the user-visible bug.
 */
export async function runClearMcpOauth(inputPath: string): Promise<number> {
  let parsed: ClearInput;
  try {
    const raw = await Bun.file(inputPath).text();
    parsed = JSON.parse(raw.trim()) as ClearInput;
  } catch (err) {
    log.error("mcp.clear.parse_failed", {
      input_path: inputPath,
      error: (err as Error).message,
    });
    return 1;
  }

  const spec = parsed.server;
  log.info("mcp.clear.start", {
    server: spec.name,
    transport: spec.transport,
    token_cache_dir: spec.token_cache_dir ?? "(default)",
  });

  try {
    await clearAllOauthState(spec);
    log.info("mcp.clear.ok", { server: spec.name });
    return 0;
  } catch (err) {
    log.error("mcp.clear.failed", {
      server: spec.name,
      error: (err as Error).message,
    });
    return 1;
  }
}

/**
 * Wipe every cache mcporter 0.10.x writes for an MCP server. Mirrors
 * `clearOAuthCaches('all')` from `mcporter/dist/oauth-persistence.js`,
 * inlined because the function isn't reachable via the package's
 * `exports` field. The three locations:
 *
 *   1. `<tokenCacheDir>/{tokens.json,client.json,code_verifier.txt,state.txt}`
 *      — our Corivo-controlled per-server directory.
 *   2. `<XDG_DATA_HOME|~/.mcporter>/credentials.json` — shared vault,
 *      one JSON file with one entry per server keyed by
 *      `<serverName>|<sha256(descriptor).slice(0,16)>`. mcporter
 *      ALWAYS writes here, on top of (1). Forgetting this is what
 *      caused the "已断开但重新安装秒连" bug.
 *   3. `<XDG_DATA_HOME|~/.mcporter>/<serverName>/` — mcporter's legacy
 *      per-server directory; only present if a user previously ran a
 *      pre-0.10 mcporter against this server. We delete it as a
 *      precaution (mcporter does the same).
 */
async function clearAllOauthState(spec: McpServerSpec): Promise<void> {
  if (spec.token_cache_dir) {
    await fs.rm(spec.token_cache_dir, { recursive: true, force: true });
  }

  const vaultPath = oauthVaultPath();
  await clearVaultEntry(vaultPath, spec);

  const legacyPerServer = path.join(mcporterDataDir(), spec.name);
  if (legacyPerServer !== spec.token_cache_dir) {
    await fs.rm(legacyPerServer, { recursive: true, force: true }).catch(() => {});
  }
}

/** Mirror of `mcporter/dist/paths.js::mcporterDir('data')`. */
function mcporterDataDir(): string {
  const xdg = process.env.XDG_DATA_HOME;
  if (xdg && xdg.trim().length > 0 && path.isAbsolute(xdg.trim())) {
    return path.join(xdg.trim(), "mcporter");
  }
  return path.join(os.homedir(), ".mcporter");
}

/** Mirror of `mcporter/dist/oauth-vault.js::getOAuthVaultPath`. */
function oauthVaultPath(): string {
  return path.join(mcporterDataDir(), "credentials.json");
}

/**
 * Compute mcporter's vault key for an MCP server. MUST stay in sync
 * with `vaultKeyForDefinition` in `mcporter/dist/oauth-vault.js`:
 *
 *   key = `${name}|${sha256(JSON.stringify(descriptor)).slice(0, 16)}`
 *
 *   descriptor = {
 *     name,
 *     url:     command.kind === 'http'  ? command.url.toString() : null,
 *     command: command.kind === 'stdio' ? { command, args }      : null,
 *   }
 *
 * The 16-char prefix matches mcporter's truncation. If mcporter ever
 * changes the schema, our key won't match and the vault entry will
 * leak — caller is responsible for noticing in QA.
 */
function vaultKeyFor(spec: McpServerSpec): string {
  let descriptor: object;
  if (spec.transport === "http") {
    // Note: mcporter stringifies via `new URL(url).toString()`, which
    // normalizes the input (lowercase scheme/host, default port stripping,
    // path normalization). Mirror that to keep keys identical.
    const normalized = new URL(spec.url).toString();
    descriptor = { name: spec.name, url: normalized, command: null };
  } else {
    descriptor = {
      name: spec.name,
      url: null,
      command: { command: spec.command, args: spec.args ?? [] },
    };
  }
  const hash = crypto
    .createHash("sha256")
    .update(JSON.stringify(descriptor))
    .digest("hex")
    .slice(0, 16);
  return `${spec.name}|${hash}`;
}

async function clearVaultEntry(
  vaultPath: string,
  spec: McpServerSpec,
): Promise<void> {
  let raw: string;
  try {
    raw = await fs.readFile(vaultPath, "utf8");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      return; // No vault file → nothing to clear.
    }
    throw err;
  }

  let parsed: { version?: number; entries?: Record<string, unknown> };
  try {
    parsed = JSON.parse(raw);
  } catch {
    // Corrupt vault — best-effort; leave it alone for mcporter to
    // overwrite on next bootstrap.
    log.warn("mcp.clear.vault_unparseable", { vault_path: vaultPath });
    return;
  }
  if (!parsed.entries) return;

  const key = vaultKeyFor(spec);
  if (!(key in parsed.entries)) return;

  delete parsed.entries[key];
  await fs.writeFile(vaultPath, JSON.stringify(parsed, null, 2), "utf8");
  log.info("mcp.clear.vault_entry_removed", {
    server: spec.name,
    vault_path: vaultPath,
    key,
  });
}

// Same translator shape as runtime.ts but inlined to keep bootstrap
// independent of agent-runner's import graph (which pulls in pi-ai
// providers, model catalog, native tools, etc. — none of which the
// bootstrap path needs).
function toServerDefinition(spec: McpServerSpec) {
  const common = {
    name: spec.name,
    tokenCacheDir: spec.token_cache_dir,
    allowedTools: spec.allowed_tools,
    blockedTools: spec.blocked_tools,
  };

  if (spec.transport === "http") {
    return {
      ...common,
      command: { kind: "http" as const, url: new URL(spec.url) },
    };
  }

  return {
    ...common,
    command: {
      kind: "stdio" as const,
      command: spec.command,
      args: spec.args ?? [],
      cwd: process.cwd(),
    },
    env: spec.env,
  };
}
