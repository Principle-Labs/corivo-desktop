//
//  ControlHandlers.swift
//  v1 control-plane handlers (ping / shutdown). Domain handlers
//  (recording.* / screen.* / ax.* / foreground.* / ocr.*) live in
//  per-feature files added in their respective phases.
//

import Foundation

enum ControlHandlers {
    /// `ping` → `{ "pong": true }`. Liveness sanity check beyond heartbeat.
    static func ping(_ request: RequestMessage) async throws {
        let resp = ResponseMessage(
            id: request.id,
            result: AnyCodable(["pong": true])
        )
        try await EventEmitter.shared.send(resp)
    }

    /// `shutdown` → `{ "ack": true }`, then exit cleanly.
    ///
    /// The 50ms sleep is the spec-aligned "let stdout drain" pause —
    /// ensures the ack is on the wire before the process tears down.
    static func shutdown(_ request: RequestMessage) async throws {
        let resp = ResponseMessage(
            id: request.id,
            result: AnyCodable(["ack": true])
        )
        try await EventEmitter.shared.send(resp)
        try? await Task.sleep(nanoseconds: 50_000_000)
        Log.info("shutdown.ack_sent_exiting")
        exit(0)
    }
}
