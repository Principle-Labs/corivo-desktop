#!/bin/sh
# Stage every sidecar Tauri's bundling expects.
#
# Two delivery shapes are now in play:
#
#   externalBin (flat Mach-O, copied next to the main executable)
#     - corivo-mcp    (packages/mcp,   Rust)
#     - corivo-agent  (packages/agent, Bun --compile)
#
#   bundle.resources (full .app bundle, copied into Contents/Resources/)
#     - CorivoCaptureHelper.app  (packages/desktop-helpers, Swift)
#       Wrapping the helper in a .app + Info.plist with LSUIElement=YES
#       is what lets TCC merge it into Corivo.app via responsible-
#       process attribution. See packages/desktop-helpers/macos/build.sh
#       for the full rationale (CO-31).
#
# This script:
#   1. seeds placeholder files at `binaries/<name>-<triple>` for the
#      two flat sidecars so tauri-build's externalBin validation can
#      run *before* the real binaries exist (chicken-and-egg). Helper
#      .app does NOT need a placeholder — bundle.resources is not
#      validated at compile time.
#   2. delegates the real builds to each package so dev iterations on
#      Swift / Rust / TS sidecar code pick up automatically when
#      `pnpm app:dev` invokes us via beforeDevCommand.
#
# Usage:
#   scripts/prep-sidecar.sh           → debug build (for `tauri dev`)
#   scripts/prep-sidecar.sh release   → release build (for `tauri build`)
#
# Target triple resolution (highest priority first):
#   1. $TARGET env var — explicit override
#   2. $TAURI_ENV_ARCH — set by Tauri's beforeBuildCommand /
#      beforeBundleCommand hooks when invoked as
#      `tauri build --target <triple>`
#   3. host triple from `rustc -vV` — covers `tauri dev` and manual
#      `cargo check`
#
# Why a placeholder dance:
#   The `tauri-build` crate runs in `corivo_app_lib`'s build.rs and
#   *validates that every externalBin path exists at compile time* —
#   even for `cargo check`. So before the real sidecar binaries exist
#   we must seed `binaries/<name>-<triple>` with anything. Subsequent
#   runs let the per-package build script overwrite the placeholders.

set -e

# Always anchor to src-tauri/ regardless of where the caller invokes us.
cd "$(dirname "$0")/.."

PROFILE="${1:-debug}"
HOST_TRIPLE="$(rustc -vV | sed -n 's/host: //p')"

if [ -n "$TARGET" ]; then
    TARGET_TRIPLE="$TARGET"
elif [ -n "$TAURI_ENV_ARCH" ]; then
    case "$TAURI_ENV_ARCH" in
        aarch64)   TARGET_TRIPLE="aarch64-apple-darwin" ;;
        x86_64)    TARGET_TRIPLE="x86_64-apple-darwin" ;;
        universal) TARGET_TRIPLE="universal-apple-darwin" ;;
        *)         TARGET_TRIPLE="$HOST_TRIPLE" ;;
    esac
else
    TARGET_TRIPLE="$HOST_TRIPLE"
fi

mkdir -p binaries

# Make sure every externalBin path exists before any cargo invocation —
# tauri-build verifies them at compile time, even for placeholder runs.
seed_placeholders() {
    triple="$1"
    for name in corivo-mcp corivo-agent; do
        path="binaries/${name}-${triple}"
        [ -e "$path" ] || touch "$path"
    done
}

if [ "$TARGET_TRIPLE" = "universal-apple-darwin" ]; then
    seed_placeholders "aarch64-apple-darwin"
    seed_placeholders "x86_64-apple-darwin"
    seed_placeholders "universal-apple-darwin"
else
    seed_placeholders "$TARGET_TRIPLE"
fi

# Anchor to the repo root so pnpm picks up the workspace.
REPO_ROOT="$(cd ../../.. && pwd)"

# Script naming asymmetry across the three sidecar packages:
#   @corivo/mcp + @corivo/agent : `build` = release, `build:debug` = debug
#   @corivo/desktop-helpers     : `build` = debug,   `build:release` = release
# Resolve to the right script name per package and per profile.
case "$PROFILE" in
    release)
        MCP_AGENT_SCRIPT="build"
        HELPERS_SCRIPT="build:release"
        ;;
    debug)
        MCP_AGENT_SCRIPT="build:debug"
        HELPERS_SCRIPT="build"
        ;;
    *)
        echo "prep-sidecar: unknown profile '$PROFILE' (expected debug|release)" >&2
        exit 1
        ;;
esac

# Propagate the macOS Developer ID to the desktop-helpers build.
#
# Tauri's codesign step only covers the main executable + externalBin
# sidecars. CorivoCaptureHelper.app ships via bundle.resources (see
# tauri.conf.json), which Tauri copies verbatim — so the helper's own
# build.sh has to do the signing. That script reads CORIVO_SIGNING_IDENTITY
# and falls back to ad-hoc + --timestamp=none when unset, which fails
# notarization ("not signed with a valid Developer ID certificate" +
# "signature does not include a secure timestamp").
#
# Source the identity from tauri.conf.json so there's a single source of
# truth. An explicit env var still wins, for ad-hoc local release builds.
if [ "$PROFILE" = "release" ] && [ "$(uname -s)" = "Darwin" ] && [ -z "${CORIVO_SIGNING_IDENTITY:-}" ]; then
    SIGNING_IDENTITY="$(node -e 'try { const c = require("./tauri.conf.json"); const id = c && c.bundle && c.bundle.macOS && c.bundle.macOS.signingIdentity; if (id) process.stdout.write(id); } catch (_) {}')"
    if [ -n "$SIGNING_IDENTITY" ]; then
        echo "prep-sidecar: exporting CORIVO_SIGNING_IDENTITY from tauri.conf.json"
        export CORIVO_SIGNING_IDENTITY="$SIGNING_IDENTITY"
    else
        echo "prep-sidecar: warning — release build but no signingIdentity in tauri.conf.json; helper will be ad-hoc signed and fail notarization" >&2
    fi
fi

# Forward the resolved target triple so each sidecar build only compiles
# the arch we actually need. Without this every Tauri build cross-compiles
# both arm64 and x86_64 even when the parent is arch-specific. All three
# build scripts accept `--target <triple>`; passing universal-apple-darwin
# (or no flag) requests the historical "build everything + lipo" path.

echo "prep-sidecar: pnpm --filter @corivo/mcp run $MCP_AGENT_SCRIPT -- --target $TARGET_TRIPLE"
( cd "$REPO_ROOT" && pnpm --filter @corivo/mcp run "$MCP_AGENT_SCRIPT" -- --target "$TARGET_TRIPLE" )

echo "prep-sidecar: pnpm --filter @corivo/agent run $MCP_AGENT_SCRIPT -- --target $TARGET_TRIPLE"
( cd "$REPO_ROOT" && pnpm --filter @corivo/agent run "$MCP_AGENT_SCRIPT" -- --target "$TARGET_TRIPLE" )

echo "prep-sidecar: pnpm --filter @corivo/desktop-helpers run $HELPERS_SCRIPT -- --target $TARGET_TRIPLE"
( cd "$REPO_ROOT" && pnpm --filter @corivo/desktop-helpers run "$HELPERS_SCRIPT" -- --target "$TARGET_TRIPLE" )
