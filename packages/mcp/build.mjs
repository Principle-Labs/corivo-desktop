#!/usr/bin/env node
//
// Build the corivo-mcp sidecar for both macOS targets, lipo into a
// universal binary, and stage all triples to apps/desktop/src-tauri/binaries/
// where Tauri's externalBin picks them up.
//
// Usage:
//   node ./build.mjs                                       # debug, all triples
//   node ./build.mjs release                               # release, all triples
//   node ./build.mjs release --target aarch64-apple-darwin # release, arm64 only
//   node ./build.mjs release --target x86_64-apple-darwin  # release, x64 only
//   node ./build.mjs release --target universal-apple-darwin # release, both + lipo
//   node ./build.mjs clean                                 # remove staged sidecars + target/
//
// Naming follows the convention enforced by tauri.conf.json `externalBin`:
//   apps/desktop/src-tauri/binaries/corivo-mcp-<target-triple>
//
// Target selection:
//   - `--target` unset → build every installed rustup target + lipo universal
//     (preserves the historical "stage all triples" behaviour for direct
//     `pnpm --filter @corivo/mcp run build` invocations).
//   - `--target <single triple>` → only that triple, no lipo. Used by
//     prep-sidecar.sh when the parent Tauri build is arch-specific so
//     we don't burn cycles cross-compiling the unused arch.
//   - `--target universal-apple-darwin` → both per-arch + lipo, same as
//     the no-flag default.
//
// Cross-compiling Rust to both arm64 and x86_64 macOS requires:
//   rustup target add aarch64-apple-darwin x86_64-apple-darwin

import { spawn, spawnSync } from "node:child_process";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  rmSync,
  unlinkSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

// argv[2] is the profile (debug|release|clean). `--target <triple>` may
// appear anywhere after that; everything else is ignored.
const argv = process.argv.slice(2);
const profile = argv[0] && !argv[0].startsWith("--") ? argv[0] : "debug";
function readFlag(name) {
  const idx = argv.indexOf(name);
  return idx >= 0 ? argv[idx + 1] : null;
}
const explicitTarget = readFlag("--target");

const stagingDir = resolve(
  here,
  "..",
  "..",
  "apps",
  "desktop",
  "src-tauri",
  "binaries",
);

const manifest = join(here, "Cargo.toml");
// Crate-local target dir keeps build artifacts out of the desktop crate's
// target/ (which has its own per-triple subdirs). Mirrors how
// packages/agent uses .build/.
const targetDir = join(here, "target");

if (profile === "clean") {
  if (existsSync(stagingDir)) {
    for (const name of readdirSync(stagingDir)) {
      if (name.startsWith("corivo-mcp-")) {
        unlinkSync(join(stagingDir, name));
      }
    }
  }
  rmSync(targetDir, { recursive: true, force: true });
  process.exit(0);
}

if (profile !== "debug" && profile !== "release") {
  console.error(
    `mcp build: unknown profile '${profile}' (expected debug|release|clean)`,
  );
  process.exit(1);
}

if (process.platform !== "darwin" && process.platform !== "win32") {
  console.warn(
    `mcp build: platform '${process.platform}' is not a Tauri target for Corivo; skipping.`,
  );
  process.exit(0);
}

mkdirSync(stagingDir, { recursive: true });

function run(cmd, args, cwd) {
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(cmd, args, { cwd, stdio: "inherit", shell: false });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`${cmd} exited with code ${code}`));
    });
  });
}

function installedTargets() {
  const result = spawnSync("rustup", ["target", "list", "--installed"], {
    encoding: "utf8",
  });
  if (result.error || result.status !== 0) {
    console.warn(
      `mcp build: could not query rustup targets — assuming host only`,
    );
    return new Set();
  }
  return new Set(
    result.stdout
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean),
  );
}

const ARM_TRIPLE = "aarch64-apple-darwin";
const X86_TRIPLE = "x86_64-apple-darwin";
const UNI_TRIPLE = "universal-apple-darwin";
const WIN_X64_TRIPLE = "x86_64-pc-windows-msvc";

if (process.platform === "win32") {
  if (explicitTarget && explicitTarget !== WIN_X64_TRIPLE) {
    console.error(
      `mcp build: --target '${explicitTarget}' is not a Windows target ` +
        `(expected ${WIN_X64_TRIPLE})`,
    );
    process.exit(1);
  }

  mkdirSync(stagingDir, { recursive: true });
  const dest = join(stagingDir, `corivo-mcp-${WIN_X64_TRIPLE}.exe`);
  if (!existsSync(dest)) closeSync(openSync(dest, "w"));
  console.warn(
    "[mcp build] Windows MCP sidecar is not built yet: packages/mcp currently uses UnixStream. " +
      `Seeded placeholder for Tauri externalBin validation -> ${dest}`,
  );
  console.log(
    `[mcp build] done (${profile}${explicitTarget ? `, target=${explicitTarget}` : ""})`,
  );
  process.exit(0);
}

const installed = installedTargets();
const haveArm = installed.has(ARM_TRIPLE);
const haveX86 = installed.has(X86_TRIPLE);

// Decide which triples to build based on --target.
// wantLipo controls whether we lipo a universal binary at the end.
let wantArm, wantX86, wantLipo;
if (!explicitTarget || explicitTarget === UNI_TRIPLE) {
  wantArm = true;
  wantX86 = true;
  wantLipo = true;
} else if (explicitTarget === ARM_TRIPLE) {
  wantArm = true;
  wantX86 = false;
  wantLipo = false;
} else if (explicitTarget === X86_TRIPLE) {
  wantArm = false;
  wantX86 = true;
  wantLipo = false;
} else {
  console.error(
    `mcp build: unknown --target '${explicitTarget}' (expected ${ARM_TRIPLE}, ${X86_TRIPLE}, or ${UNI_TRIPLE})`,
  );
  process.exit(1);
}

if (wantArm && !haveArm) {
  if (explicitTarget) {
    console.error(
      `mcp build: --target ${ARM_TRIPLE} but rustup target not installed.\n` +
        `  Install with: rustup target add ${ARM_TRIPLE}`,
    );
    process.exit(1);
  }
  console.warn(
    `mcp build: target '${ARM_TRIPLE}' missing — skipping (rustup target add ${ARM_TRIPLE})`,
  );
  wantArm = false;
}
if (wantX86 && !haveX86) {
  if (explicitTarget) {
    console.error(
      `mcp build: --target ${X86_TRIPLE} but rustup target not installed.\n` +
        `  Install with: rustup target add ${X86_TRIPLE}`,
    );
    process.exit(1);
  }
  console.warn(
    `mcp build: target '${X86_TRIPLE}' missing — skipping (rustup target add ${X86_TRIPLE})`,
  );
  wantX86 = false;
}
if (!wantArm && !wantX86) {
  console.error(
    `mcp build: no buildable triple for target '${explicitTarget ?? "<host>"}'.\n` +
      `  Install with: rustup target add ${ARM_TRIPLE} ${X86_TRIPLE}`,
  );
  process.exit(1);
}

const cargoFlags = profile === "release" ? ["--release"] : [];
const profileDir = profile === "release" ? "release" : "debug";

async function buildTriple(triple) {
  console.log(`[mcp build] cargo build --bin corivo-mcp --target ${triple} (${profile})`);
  await run(
    "cargo",
    [
      "build",
      "--bin",
      "corivo-mcp",
      "--manifest-path",
      manifest,
      "--target-dir",
      targetDir,
      "--target",
      triple,
      ...cargoFlags,
    ],
    here,
  );
  return join(targetDir, triple, profileDir, "corivo-mcp");
}

const built = [];
if (wantArm) built.push([ARM_TRIPLE, await buildTriple(ARM_TRIPLE)]);
if (wantX86) built.push([X86_TRIPLE, await buildTriple(X86_TRIPLE)]);

// Stage per-triple binaries.
for (const [triple, src] of built) {
  const dest = join(stagingDir, `corivo-mcp-${triple}`);
  await run("cp", [src, dest], here);
  await run("chmod", ["+x", dest], here);
  console.log(`[mcp build] -> ${dest}`);
}

// Lipo the universal target only when we built both per-arch binaries
// AND the caller actually wants a universal artifact (default = yes,
// arch-specific --target = no).
if (wantLipo && wantArm && wantX86) {
  const uniDest = join(stagingDir, `corivo-mcp-${UNI_TRIPLE}`);
  const armBin = built.find(([t]) => t === ARM_TRIPLE)[1];
  const x86Bin = built.find(([t]) => t === X86_TRIPLE)[1];
  console.log(`[mcp build] lipo -create -> ${uniDest}`);
  await run("lipo", ["-create", "-output", uniDest, armBin, x86Bin], here);
  await run("chmod", ["+x", uniDest], here);
  console.log(`[mcp build] -> ${uniDest}`);
} else if (wantLipo) {
  console.warn(
    `mcp build: only one target installed — skipping lipo of corivo-mcp-${UNI_TRIPLE}`,
  );
}

console.log(
  `[mcp build] done (${profile}${explicitTarget ? `, target=${explicitTarget}` : ""})`,
);
