//
//  Heartbeat.swift
//  Periodic liveness emit. Spec § 5.4 default cadence: every 5 seconds.
//
//  Cadence can be shortened via `CORIVO_HELPER_HEARTBEAT_INTERVAL_MS`
//  for tests that need to exercise health-monitor transitions in seconds
//  rather than 15-second windows.
//

import Foundation

enum Heartbeat {
    /// Default cadence per spec.
    private static let defaultIntervalMs: UInt64 = 5_000

    /// Once started, runs until the helper exits (no stop API needed —
    /// the task is detached and goes away with the process).
    static func start() {
        let intervalMs = parseIntervalOverride() ?? defaultIntervalMs
        Task.detached(priority: .background) {
            while !Task.isCancelled {
                let nanos = intervalMs * 1_000_000
                try? await Task.sleep(nanoseconds: nanos)
                let ts = Self.iso8601(Date())
                let hb = HeartbeatMessage(ts: ts)
                do {
                    try await EventEmitter.shared.send(hb)
                } catch {
                    Log.warn("heartbeat write failed (will retry next tick): \(error)")
                }
            }
        }
    }

    private static func parseIntervalOverride() -> UInt64? {
        guard
            let raw = ProcessInfo.processInfo.environment["CORIVO_HELPER_HEARTBEAT_INTERVAL_MS"],
            let n = UInt64(raw),
            n > 0
        else {
            return nil
        }
        return n
    }

    static func iso8601(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: date)
    }
}
