#!/usr/bin/env node
// Cross-platform replacement for prep-sidecar.sh.
//
// Tauri validates `externalBin` paths before the real sidecars exist, so
// this script first seeds placeholder files and then builds the actual
// sidecars from the workspace. On macOS we delegate to the original bash
// script so existing signing / lipo flows stay untouched; on Windows we
// call each package build script directly.

import { execFileSync, spawnSync } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC_TAURI = resolve(HERE, "..");
const REPO_ROOT = resolve(SRC_TAURI, "..", "..", "..");
const profile = process.argv[2] === "release" ? "release" : "debug";

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = out.match(/^host:\s*(.+)$/m);
  if (!m) throw new Error(`prep-sidecar: could not parse rustc host triple from:\n${out}`);
  return m[1].trim();
}

function resolveTriple() {
  if (process.env.TARGET) return process.env.TARGET;
  if (process.env.TAURI_ENV_ARCH) {
    const arch = process.env.TAURI_ENV_ARCH;
    if (process.platform === "darwin") {
      if (arch === "aarch64") return "aarch64-apple-darwin";
      if (arch === "x86_64") return "x86_64-apple-darwin";
      if (arch === "universal") return "universal-apple-darwin";
    }
  }
  return hostTriple();
}

function seedPlaceholders(triple) {
  const binariesDir = join(SRC_TAURI, "binaries");
  if (!existsSync(binariesDir)) mkdirSync(binariesDir, { recursive: true });
  const exeSuffix = triple.includes("windows") ? ".exe" : "";
  // Sidecars staged at binaries/<name>-<triple>{.exe}. tauri-build's
  // externalBin validator only cares about file existence at compile
  // time; the per-sidecar build script overwrites these placeholders
  // with the real binary later.
  //
  // The Windows capture helper is seeded only on Windows: on macOS the
  // capture helper ships as a .app bundle via `bundle.resources`, not
  // externalBin, so the flat binary path is irrelevant there.
  const names = triple.includes("windows")
    ? ["corivo-mcp", "corivo-agent", "corivo-capture-helper"]
    : ["corivo-mcp", "corivo-agent"];
  for (const name of names) {
    const path = join(binariesDir, `${name}-${triple}${exeSuffix}`);
    if (!existsSync(path)) {
      // Placeholder file — content doesn't matter, only existence.
      closeSync(openSync(path, "w"));
    }
  }
  // tauri-build also validates every `bundle.resources` path. The
  // helper .app is a macOS-only artifact built by
  // packages/desktop-helpers/macos/build.sh, which doesn't run on
  // Windows/Linux. Seed a placeholder dir so the existence check
  // passes; nothing reads from it in `tauri dev` (no bundling step).
  const helperPath = join(
    SRC_TAURI,
    "binaries",
    "CorivoCaptureHelper.app",
    "Contents",
    "MacOS"
  );
  if (!existsSync(helperPath)) mkdirSync(helperPath, { recursive: true });
}

const triple = resolveTriple();

if (triple === "universal-apple-darwin") {
  seedPlaceholders("aarch64-apple-darwin");
  seedPlaceholders("x86_64-apple-darwin");
  seedPlaceholders("universal-apple-darwin");
} else {
  seedPlaceholders(triple);
}

function runPnpm(filter, script, target) {
  console.log(`prep-sidecar: pnpm --filter ${filter} run ${script} -- --target ${target}`);
  const res = spawnSync(
    "pnpm",
    ["--filter", filter, "run", script, "--", "--target", target],
    {
      stdio: "inherit",
      cwd: REPO_ROOT,
      shell: process.platform === "win32",
    },
  );
  if (res.status !== 0) process.exit(res.status ?? 1);
}

// On macOS, run the original bash script so the existing signing/lipo
// path stays untouched. On Windows, build the sidecars directly from
// this workspace; placeholders are only a preflight for tauri-build's
// externalBin validator, not the final artifact.
if (process.platform === "darwin") {
  const bashScript = join(SRC_TAURI, "scripts", "prep-sidecar.sh");
  const res = spawnSync("sh", [bashScript, profile], {
    stdio: "inherit",
    cwd: SRC_TAURI,
  });
  if (res.status !== 0) process.exit(res.status ?? 1);
} else if (process.platform === "win32") {
  const mcpAgentScript = profile === "release" ? "build" : "build:debug";
  const helpersScript = profile === "release" ? "build:release" : "build";
  runPnpm("@corivo/mcp", mcpAgentScript, triple);
  runPnpm("@corivo/agent", mcpAgentScript, triple);
  runPnpm("@corivo/desktop-helpers", helpersScript, triple);
} else {
  console.log(
    `prep-sidecar: ${process.platform} — seeded placeholder binaries for ${triple}, skipping macOS sidecar builds.`
  );
}
