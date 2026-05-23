#!/usr/bin/env bash
# PoC 验证脚本。一条命令跑完所有验证项。
# 退出码 0 = 全部通过。

set -euo pipefail

cd "$(dirname "$0")"
ROOT="$PWD"
BUILD="$ROOT/build"
rm -rf "$BUILD"
mkdir -p "$BUILD"

echo "==> [1/4] build connector bundle (esbuild via bun build, NOT compile)"
bun build "$ROOT/connector/send.ts" \
  --target=bun \
  --outfile "$BUILD/bundle.js"

echo "==> [2/4] copy broken.js (hand-written invalid JS)"
cp "$ROOT/connector/broken.js" "$BUILD/broken.js"

echo "==> [3/4] compile host binary (bun --compile, target current arch)"
bun build "$ROOT/host/main.ts" \
  --compile \
  --outfile "$BUILD/host-bin"

# Move the bundle to a path that did NOT exist at compile time, to prove the
# host binary is genuinely doing runtime resolution (not embedding the bundle).
RUNTIME_DIR="$BUILD/runtime-only"
mkdir -p "$RUNTIME_DIR"
mv "$BUILD/bundle.js" "$RUNTIME_DIR/bundle.js"
mv "$BUILD/broken.js" "$RUNTIME_DIR/broken.js"

echo
echo "==> [4/4] run verifications"
echo

echo "--- V1+V2+V3: load bundle, call tool, host callback works ---"
"$BUILD/host-bin" "$RUNTIME_DIR/bundle.js" send '{"to":"alice@example.com","body":"hello"}'
echo

echo "--- V4: import missing path → caught ---"
if "$BUILD/host-bin" "$RUNTIME_DIR/does-not-exist.js" send '{}'; then
  echo "FAIL: expected nonzero exit for missing bundle"
  exit 99
else
  echo "OK: missing path produced nonzero exit (caught)"
fi
echo

echo "--- V5: import syntactically broken .js → caught ---"
if "$BUILD/host-bin" "$RUNTIME_DIR/broken.js" send '{}'; then
  echo "FAIL: expected nonzero exit for broken bundle"
  exit 98
else
  echo "OK: broken bundle produced nonzero exit (caught)"
fi
echo

echo "==> ALL PoC CHECKS PASSED"
