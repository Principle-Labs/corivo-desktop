#!/usr/bin/env node
//
// Compile the @corivo/agent sidecar with `bun build --compile` for the
// host platform's Tauri triples and stage them to
// apps/desktop/src-tauri/binaries/ where Tauri's externalBin picks them up.
//
// Usage:
//   node ./build.mjs                                       # release, all triples
//   node ./build.mjs debug                                 # debug, all triples
//   node ./build.mjs release --target aarch64-apple-darwin # release, arm64 only
//   node ./build.mjs release --target x86_64-apple-darwin  # release, x64 only
//   node ./build.mjs release --target universal-apple-darwin # release, both + lipo
//   node ./build.mjs release --target x86_64-pc-windows-msvc # release, win64 only
//   node ./build.mjs clean                                 # remove staged binaries + .build
//
// Naming follows the convention enforced by tauri.conf.json `externalBin`:
//   apps/desktop/src-tauri/binaries/corivo-agent-<target-triple>[.exe]
//
// Target selection:
//   - On macOS: no `--target` → both arches + universal lipo. `--target`
//     a single apple-darwin triple → only that, no lipo. Used by
//     prep-sidecar.sh when the parent Tauri build is arch-specific.
//   - On Windows: no `--target` → x86_64-pc-windows-msvc (the canonical
//     Tauri triple). `--target x86_64-pc-windows-msvc` is also accepted
//     (no-op). Other triples are rejected with a friendly error.

import { spawn, spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  rmSync,
  unlinkSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

const argv = process.argv.slice(2);
const profile = argv[0] && !argv[0].startsWith("--") ? argv[0] : "release";
function readFlag(name) {
  const idx = argv.indexOf(name);
  return idx >= 0 ? argv[idx + 1] : null;
}
const explicitTarget = readFlag("--target");

const ARM_TRIPLE = "aarch64-apple-darwin";
const X86_TRIPLE = "x86_64-apple-darwin";
const UNI_TRIPLE = "universal-apple-darwin";
const WIN_X64_TRIPLE = "x86_64-pc-windows-msvc";

const stagingDir = resolve(
  here,
  "..",
  "..",
  "apps",
  "desktop",
  "src-tauri",
  "binaries",
);

const buildDir = join(here, ".build");

if (profile === "clean") {
  if (existsSync(stagingDir)) {
    for (const name of readdirSync(stagingDir)) {
      if (name.startsWith("corivo-agent-")) {
        unlinkSync(join(stagingDir, name));
      }
    }
  }
  rmSync(buildDir, { recursive: true, force: true });
  process.exit(0);
}

if (profile !== "debug" && profile !== "release") {
  console.error(
    `agent build: unknown profile '${profile}' (expected debug|release|clean)`,
  );
  process.exit(1);
}

function run(cmd, args, cwd) {
  return new Promise((resolvePromise, rejectPromise) => {
    // bun ships as bun.exe on Windows; Node's `spawn` with `shell: false`
    // doesn't apply the PATHEXT lookup, so spawning bare "bun" fails with
    // ENOENT even when bun.exe is on PATH. Flip on `shell: true` on
    // Windows so the shell handles the .exe / .ps1 extension resolution.
    const useShell = process.platform === "win32";
    const child = spawn(cmd, args, { cwd, stdio: "inherit", shell: useShell });
    child.on("error", rejectPromise);
    child.on("exit", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`${cmd} exited with code ${code}`));
    });
  });
}

// Preflight: bun on PATH? On Windows the official `irm bun.com/install.ps1`
// installer writes `%USERPROFILE%\.bun\bin` to the user-level PATH, but
// the running shell (and anything it spawned, like turbo) doesn't pick
// up the new value until it restarts. Surface a clear instruction
// instead of letting cmd emit the cryptic
// `'bun' is not recognized as an internal or external command`.
function preflightBun() {
  // `shell: true` on Windows so PATHEXT resolves bun → bun.exe. Without
  // it, `spawnSync("bun")` does CreateProcess on the literal "bun" and
  // doesn't try the .exe suffix.
  const probe = spawnSync("bun", ["--version"], {
    stdio: ["ignore", "ignore", "ignore"],
    shell: process.platform === "win32",
  });
  if (probe.status === 0) return;

  // Fallback: maybe bun.exe IS installed, just not visible to this
  // shell's PATH. Detect that case and tell the user how to refresh.
  if (process.platform === "win32") {
    const bundled = join(homedir(), ".bun", "bin", "bun.exe");
    if (existsSync(bundled)) {
      console.error(
        `agent build: bun.exe is installed at ${bundled} but isn't on this shell's PATH.\n` +
          `  → Restart your terminal, OR run in the current PowerShell session:\n` +
          `      $env:Path = [Environment]::GetEnvironmentVariable('Path','User') + ';' + $env:Path\n` +
          `    then re-run the build.`,
      );
      process.exit(1);
    }
    console.error(
      `agent build: bun is required but not found on PATH.\n` +
        `  → Install bun (Windows, no admin):\n` +
        `      powershell -c "irm bun.com/install.ps1 | iex"\n` +
        `    Then restart your terminal so PATH picks up %USERPROFILE%\\.bun\\bin.`,
    );
    process.exit(1);
  }

  console.error(
    `agent build: bun is required but not found on PATH.\n` +
      `  → Install bun (macOS / Linux):\n` +
      `      curl -fsSL https://bun.sh/install | bash`,
  );
  process.exit(1);
}
preflightBun();

mkdirSync(stagingDir, { recursive: true });
mkdirSync(buildDir, { recursive: true });

const minify = profile === "release" ? ["--minify"] : [];
const sourceEntry = "src/main.ts";

// Per-platform output naming: Tauri externalBin on Windows expects the
// `.exe` suffix; macOS / Linux don't append anything.
const exeSuffix = process.platform === "win32" ? ".exe" : "";

async function bunCompile(triple, bunTarget) {
  const outfile = join(buildDir, `corivo-agent-${triple}${exeSuffix}`);
  console.log(`[agent build] bun --compile ${profile} → ${bunTarget}`);
  await run(
    "bun",
    [
      "build",
      sourceEntry,
      "--compile",
      `--target=${bunTarget}`,
      ...minify,
      "--outfile",
      outfile,
    ],
    here,
  );
  return outfile;
}

function stageBinary(triple, src) {
  const dest = join(stagingDir, `corivo-agent-${triple}${exeSuffix}`);
  copyFileSync(src, dest);
  console.log(`[agent build] -> ${dest}`);
}

if (process.platform === "darwin") {
  // ----- macOS path: same as before — arm64 + x64 + optional lipo.
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
      `agent build: unknown --target '${explicitTarget}' for macOS ` +
        `(expected ${ARM_TRIPLE}, ${X86_TRIPLE}, or ${UNI_TRIPLE})`,
    );
    process.exit(1);
  }

  const arm = wantArm ? await bunCompile(ARM_TRIPLE, "bun-darwin-arm64") : null;
  const x86 = wantX86 ? await bunCompile(X86_TRIPLE, "bun-darwin-x64") : null;

  let universal = null;
  if (wantLipo && arm && x86) {
    universal = join(buildDir, `corivo-agent-${UNI_TRIPLE}`);
    console.log(`[agent build] lipo -create → universal binary`);
    await run("lipo", ["-create", "-output", universal, arm, x86], here);
  }

  if (arm) {
    stageBinary(ARM_TRIPLE, arm);
    await run("chmod", ["+x", join(stagingDir, `corivo-agent-${ARM_TRIPLE}`)], here);
  }
  if (x86) {
    stageBinary(X86_TRIPLE, x86);
    await run("chmod", ["+x", join(stagingDir, `corivo-agent-${X86_TRIPLE}`)], here);
  }
  if (universal) {
    stageBinary(UNI_TRIPLE, universal);
    await run("chmod", ["+x", join(stagingDir, `corivo-agent-${UNI_TRIPLE}`)], here);
  }
} else if (process.platform === "win32") {
  // ----- Windows path: single x64 build.
  if (explicitTarget && explicitTarget !== WIN_X64_TRIPLE) {
    console.error(
      `agent build: --target '${explicitTarget}' is not a Windows target ` +
        `(expected ${WIN_X64_TRIPLE})`,
    );
    process.exit(1);
  }
  const out = await bunCompile(WIN_X64_TRIPLE, "bun-windows-x64");
  stageBinary(WIN_X64_TRIPLE, out);
} else {
  console.warn(
    `agent build: platform '${process.platform}' is not a Tauri target for Corivo; skipping.`,
  );
  process.exit(0);
}

console.log(
  `[agent build] done (${profile}${explicitTarget ? `, target=${explicitTarget}` : ""})`,
);
