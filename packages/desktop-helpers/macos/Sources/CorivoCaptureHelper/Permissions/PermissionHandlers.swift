//
//  PermissionHandlers.swift
//  Phase 7 — RPC handler for `permission.status`. Reports the system
//  permission state for the helper bundle on macOS:
//
//    - microphone        → AVCaptureDevice.authorizationStatus
//    - screen_recording  → CGPreflightScreenCaptureAccess (no-prompt)
//    - accessibility     → AXIsProcessTrusted (no-prompt)
//
//  All three are non-prompting reads; safe to call from a settings page
//  render path.
//

import AVFoundation
import ApplicationServices
import CoreGraphics
import Foundation

enum PermissionHandlers {

    static func status(_ request: RequestMessage) async throws {
        let mic = currentMicrophoneStatus()
        let screen = currentScreenRecordingStatus()
        let ax = currentAccessibilityStatus()

        let json: [String: Any] = [
            "microphone": mic,
            "screen_recording": screen,
            "accessibility": ax,
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    private static func currentMicrophoneStatus() -> String {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:                return "granted"
        case .denied, .restricted:       return "denied"
        case .notDetermined:             return "undetermined"
        @unknown default:                return "undetermined"
        }
    }

    private static func currentScreenRecordingStatus() -> String {
        // CGPreflightScreenCaptureAccess (macOS 10.15+) returns true
        // only when the helper bundle has been granted screen recording
        // permission. It does NOT trigger the permission dialog.
        return CGPreflightScreenCaptureAccess() ? "granted" : "undetermined"
    }

    private static func currentAccessibilityStatus() -> String {
        return AXIsProcessTrusted() ? "granted" : "undetermined"
    }
}
