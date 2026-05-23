// swift-tools-version: 5.9
//
// CorivoCaptureHelper
// ====================
// Sidecar binary that the Rust capture_client spawns. Implements the
// cross-platform v1 protocol defined in:
//   apps/desktop/src-tauri/schemas/capture-helper-v1.json
//   apps/desktop/docs/capture-helper-architecture-spec.md
//   apps/desktop/docs/capture-helper-macos-spec.md
//
// Phase 0 scope (this directory): control plane only — hello / heartbeat /
// ping / shutdown / `--echo-mode`. Audio / screen / accessibility / OCR are
// added under their respective phases per the macOS spec.

import PackageDescription

let package = Package(
    name: "CorivoCaptureHelper",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "CorivoCaptureHelper",
            path: "Sources/CorivoCaptureHelper"
        )
    ]
)
