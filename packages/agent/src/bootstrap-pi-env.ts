// Side-effect-only module — MUST be the very first import in main.ts.
//
// Why this exists:
//   `@mariozechner/pi-coding-agent`'s `dist/config.js` executes a top-
//   level statement
//
//       const pkg = JSON.parse(readFileSync(getPackageJsonPath(), "utf-8"));
//
//   so the moment ANY transitive import touches that package (we hit it
//   via skills.ts, native-tools/index.ts), the file read happens. Inside
//   a `bun build --compile` binary `getPackageDir()` returns
//   `dirname(process.execPath)` (i.e. the directory the .Mach-O lives
//   in) — which for us is `target/debug/` in dev and
//   `Corivo.app/Contents/MacOS/` in prod. Neither has a package.json
//   next to corivo-agent, so the read throws ENOENT and the sidecar
//   exits before we can stream anything back.
//
//   pi's own `pi` CLI binary works because their `copy-binary-assets`
//   build step drops a real `package.json` (and theme assets) next to
//   the compiled binary. We can't replicate that through Tauri's
//   `externalBin` flow — externalBin copies a single Mach-O, no
//   resource sidecar.
//
// Fix:
//   Honor pi's documented escape hatch: set `PI_PACKAGE_DIR` env var.
//   `getPackageDir()` short-circuits to that path before any
//   process.execPath logic runs. We synthesize a tmp directory at
//   startup containing a minimal package.json. The fields pi reads are
//   `name`, `version`, `piConfig.{name,configDir}` — all only consumed
//   by interactive-mode CLI surfaces we don't use, so the values are
//   inert from our perspective.
//
// Why a separate file instead of inlining at the top of main.ts:
//   ESM hoists `import` declarations above all top-level statements, so
//   `process.env.PI_PACKAGE_DIR = ...` written at the top of main.ts
//   would still run AFTER pi-coding-agent's config.js. Putting the side
//   effect inside a leaf module imported FIRST forces evaluation order
//   (depth-first: this module has no further imports, so its body
//   completes before main.ts moves on to the next import).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

function bootstrap(): void {
  // Respect explicit user override, e.g. for Nix store layouts where the
  // operator drops their own pi config dir.
  if (process.env.PI_PACKAGE_DIR) return;

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "corivo-pi-package-"));
  const pkg = {
    name: "@corivo/agent",
    version: "0.0.1",
    // Inert from our usage — `loadSkills` / `createCodingTools` take
    // their dirs as explicit args. These values just have to exist so
    // pi's config.js doesn't NPE on `pkg.piConfig?.configDir`.
    piConfig: {
      name: "corivo-agent",
      configDir: ".agents",
    },
  };
  fs.writeFileSync(
    path.join(dir, "package.json"),
    JSON.stringify(pkg),
    "utf-8",
  );
  process.env.PI_PACKAGE_DIR = dir;
}

bootstrap();
