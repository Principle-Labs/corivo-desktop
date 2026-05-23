#!/usr/bin/env bash
#
# Build CorivoCaptureHelper as a macOS .app bundle and stage it where
# the desktop crate expects it.
#
# Usage:
#   ./build.sh                                              # debug, both arches + lipo
#   ./build.sh release                                      # release, both arches + lipo
#   ./build.sh release aarch64-apple-darwin                 # release, arm64 only
#   ./build.sh release x86_64-apple-darwin                  # release, x64 only
#   ./build.sh release universal-apple-darwin               # release, both + lipo
#
# Identity: $CORIVO_SIGNING_IDENTITY drives codesign; falls back to ad-hoc
# if unset.
#
# Target argument:
#   - empty / universal-apple-darwin → build arm64 + x86_64, lipo into a
#     universal Mach-O (current default for direct invocations).
#   - aarch64-apple-darwin → arm64 only, no lipo, ship single-arch Mach-O.
#   - x86_64-apple-darwin → x64 only, same.
#
#   prep-sidecar.sh forwards this when the parent Tauri build is
#   arch-specific so we skip the unused swift-build pass.
#
# Why a .app bundle and not a flat Mach-O:
#   The helper links AppKit / AVFoundation / ScreenCaptureKit. Without
#   an Info.plist declaring LSUIElement=YES, Launch Services treats it
#   as a regular GUI app and bounces its icon in the Dock forever
#   (CO-31). Wrapping it in a .app + Info.plist also gives us a stable
#   CFBundleIdentifier, which is what TCC needs to attribute the
#   helper's API calls back to the parent Corivo.app via responsible-
#   process inheritance.
#
# Output layout:
#   binaries/CorivoCaptureHelper.app/
#     Contents/
#       Info.plist                (4 keys: id/name/exec/LSUIElement)
#       MacOS/CorivoCaptureHelper (universal arm64+x86_64 Mach-O)
#       _CodeSignature/           (filled in by codesign)
#
# The output path is fixed (no triple suffix) because Tauri picks it up
# via `bundle.resources` rather than `externalBin`.

set -euo pipefail

cd "$(dirname "$0")"

CONFIG="${1:-debug}"
case "$CONFIG" in
  debug|release) ;;
  *) echo "build.sh: unknown config '$CONFIG' (expected debug|release)" >&2; exit 1 ;;
esac

TARGET="${2:-}"
case "$TARGET" in
  ""|universal-apple-darwin) BUILD_ARM=1; BUILD_X86=1 ;;
  aarch64-apple-darwin)      BUILD_ARM=1; BUILD_X86=0 ;;
  x86_64-apple-darwin)       BUILD_ARM=0; BUILD_X86=1 ;;
  *)
    echo "build.sh: unknown target '$TARGET' (expected aarch64-apple-darwin, x86_64-apple-darwin, universal-apple-darwin, or empty)" >&2
    exit 1
    ;;
esac

DEST_BINARIES_REL="../../../apps/desktop/src-tauri/binaries"
mkdir -p "$DEST_BINARIES_REL"
DEST_BINARIES="$(cd "$DEST_BINARIES_REL" && pwd)"
APP_BUNDLE_DEST="$DEST_BINARIES/CorivoCaptureHelper.app"
INFO_PLIST_TEMPLATE="$(pwd)/HelperInfo.plist"
ENTITLEMENTS="$(pwd)/Helper.entitlements"

if [ ! -f "$INFO_PLIST_TEMPLATE" ]; then
  echo "[build.sh] missing Info.plist template at $INFO_PLIST_TEMPLATE" >&2
  exit 1
fi
if [ ! -f "$ENTITLEMENTS" ]; then
  echo "[build.sh] missing entitlements at $ENTITLEMENTS" >&2
  exit 1
fi

# 1. Per-arch SwiftPM builds + (optional) lipo. The single-invocation
#    `swift build --arch arm64 --arch x86_64` form requires Xcode's
#    xcbuild, which isn't always installed (e.g. CI images with only
#    Command Line Tools). The two-step approach works with the bare
#    CLT toolchain.
ARM_BIN=".build/arm64-apple-macosx/${CONFIG}/CorivoCaptureHelper"
X86_BIN=".build/x86_64-apple-macosx/${CONFIG}/CorivoCaptureHelper"

if [ "$BUILD_ARM" = "1" ]; then
  echo "[build.sh] swift build -c $CONFIG --arch arm64"
  swift build -c "$CONFIG" --arch arm64
  if [ ! -f "$ARM_BIN" ]; then
    echo "[build.sh] missing per-arch build at $ARM_BIN" >&2
    find .build -name CorivoCaptureHelper -type f >&2 || true
    exit 1
  fi
fi

if [ "$BUILD_X86" = "1" ]; then
  echo "[build.sh] swift build -c $CONFIG --arch x86_64"
  swift build -c "$CONFIG" --arch x86_64
  if [ ! -f "$X86_BIN" ]; then
    echo "[build.sh] missing per-arch build at $X86_BIN" >&2
    find .build -name CorivoCaptureHelper -type f >&2 || true
    exit 1
  fi
fi

# Decide the Mach-O that ships inside CorivoCaptureHelper.app:
#   both arches built → lipo into universal
#   only one arch built → that single-arch binary ships as-is
if [ "$BUILD_ARM" = "1" ] && [ "$BUILD_X86" = "1" ]; then
  mkdir -p .build/universal
  HELPER_BIN=".build/universal/CorivoCaptureHelper"
  echo "[build.sh] lipo -create -> $HELPER_BIN"
  lipo -create -output "$HELPER_BIN" "$ARM_BIN" "$X86_BIN"
elif [ "$BUILD_ARM" = "1" ]; then
  HELPER_BIN="$ARM_BIN"
  echo "[build.sh] single-arch (arm64) -> $HELPER_BIN"
else
  HELPER_BIN="$X86_BIN"
  echo "[build.sh] single-arch (x86_64) -> $HELPER_BIN"
fi

# 2. Assemble the .app bundle structure into .build/CorivoCaptureHelper.app
#    first, then atomically swap into the staging directory. Doing it
#    in-place inside the destination would risk codesign seeing a
#    half-written bundle if something fails midway.
STAGING_APP=".build/CorivoCaptureHelper.app"
rm -rf "$STAGING_APP"
mkdir -p "$STAGING_APP/Contents/MacOS"
cp "$HELPER_BIN" "$STAGING_APP/Contents/MacOS/CorivoCaptureHelper"
chmod +x "$STAGING_APP/Contents/MacOS/CorivoCaptureHelper"
cp "$INFO_PLIST_TEMPLATE" "$STAGING_APP/Contents/Info.plist"

# 3. Code-sign the bundle. We run with --options runtime (Hardened
#    Runtime) in both debug and release so dev parity matches the
#    notarised release: any entitlement that breaks runtime here will
#    also break in production.
#
#    Identity selection:
#      - $CORIVO_SIGNING_IDENTITY set    → use it (release flow)
#      - otherwise                       → ad-hoc ('-')
#    Ad-hoc signing is enough for local dev; only Gatekeeper /
#    notarisation rejects ad-hoc.
SIGN_IDENTITY="${CORIVO_SIGNING_IDENTITY:--}"
if [ "$SIGN_IDENTITY" = "-" ]; then
  echo "[build.sh] codesign --sign - (ad-hoc)"
  TIMESTAMP_FLAG=("--timestamp=none")
else
  echo "[build.sh] codesign --sign '$SIGN_IDENTITY'"
  TIMESTAMP_FLAG=("--timestamp")
fi
codesign --force \
  --options runtime \
  "${TIMESTAMP_FLAG[@]}" \
  --entitlements "$ENTITLEMENTS" \
  --sign "$SIGN_IDENTITY" \
  "$STAGING_APP"

# 4. Atomic swap into binaries/. rsync --delete on the .app dir would
#    work too, but rm-then-cp is simpler and the staging path is small.
mkdir -p "$DEST_BINARIES"
rm -rf "$APP_BUNDLE_DEST"
cp -R "$STAGING_APP" "$APP_BUNDLE_DEST"

# 5. Sanity-check the output. `codesign -dv` confirms signature
#    metadata; `file` confirms the universal binary survived lipo.
echo "[build.sh] verifying signature on $APP_BUNDLE_DEST"
codesign -dv --verbose=2 "$APP_BUNDLE_DEST" 2>&1 | sed 's/^/[codesign] /'
echo "[build.sh] file: $(file -b "$APP_BUNDLE_DEST/Contents/MacOS/CorivoCaptureHelper")"
echo "[build.sh] -> $APP_BUNDLE_DEST"
