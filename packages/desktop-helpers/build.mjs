#!/usr/bin/env node
//
// Cross-platform dispatcher for the corivo-capture-helper sidecar build.
// Selects macos/build.sh or windows/build.ps1 based on the host platform.
//
// Output convention:
//   macOS  → apps/desktop/src-tauri/binaries/CorivoCaptureHelper.app/
//            consumed via tauri.conf.json `bundle.resources`, NOT
//            `externalBin`. The .app gets the Info.plist with
//            LSUIElement=YES and is the unit Tauri ships into
//            Corivo.app/Contents/Resources/ for TCC responsible-
//            process attribution.
//   win32  → apps/desktop/src-tauri/binaries/corivo-capture-helper-<triple>.exe
//            still flat, still externalBin; Windows has no Dock.
//
// Usage:
//   node ./build.mjs                                       # debug, both arches
//   node ./build.mjs release                               # release, both arches
//   node ./build.mjs release --target aarch64-apple-darwin # release, arm64 only
//   node ./build.mjs release --target x86_64-apple-darwin  # release, x64 only
//   node ./build.mjs release --target universal-apple-darwin # release, both + lipo
//   node ./build.mjs clean                                 # remove staged sidecars
//
// Target selection mirrors packages/{mcp,agent}/build.mjs and is forwarded
// to the platform-specific build script (macos/build.sh today, eventual
// windows/build.ps1). Single-triple builds skip lipo and ship a single-arch
// Mach-O inside CorivoCaptureHelper.app's MacOS/.
//
import { spawn } from "node:child_process";
import { existsSync, rmSync, readdirSync, statSync, unlinkSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

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

if (profile === "clean") {
  if (existsSync(stagingDir)) {
    for (const name of readdirSync(stagingDir)) {
      const full = join(stagingDir, name);
      // macOS: remove the helper .app bundle.
      if (name === "CorivoCaptureHelper.app") {
        rmSync(full, { recursive: true, force: true });
        continue;
      }
      // Legacy macOS flat-binary outputs from before the .app refactor.
      // Windows flat-binary outputs (still in use). Both are matched by
      // the same prefix.
      if (name.startsWith("corivo-capture-helper-")) {
        if (statSync(full).isDirectory()) {
          rmSync(full, { recursive: true, force: true });
        } else {
          unlinkSync(full);
        }
      }
    }
  }
  rmSync(join(here, "macos", ".build"), { recursive: true, force: true });
  rmSync(join(here, "windows", "build"), { recursive: true, force: true });
  process.exit(0);
}

if (profile !== "debug" && profile !== "release") {
  console.error(
    `desktop-helpers: unknown profile '${profile}' (expected debug|release|clean)`,
  );
  process.exit(1);
}

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

const platform = process.platform;
try {
  if (platform === "darwin") {
    // build.sh accepts: <profile> [target-triple]
    // Empty/missing target = both arches + lipo (current default).
    const args = explicitTarget ? [profile, explicitTarget] : [profile];
    await run("./build.sh", args, join(here, "macos"));
  } else if (platform === "win32") {
    const psProfile = profile === "release" ? "Release" : "Debug";
    await run(
      "powershell.exe",
      ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "./build.ps1", psProfile],
      join(here, "windows"),
    );
  } else {
    console.warn(
      `desktop-helpers: platform '${platform}' is not a Tauri target for Corivo; skipping.`,
    );
  }
} catch (err) {
  console.error(`desktop-helpers: build failed: ${err.message}`);
  process.exit(1);
}
