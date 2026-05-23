//
//  AudioRecorder.swift
//  Top-level recording session manager. Phase 5 v1 records into a single
//  segment per session — segment rotation lands in a follow-up refinement
//  (the protocol surface already supports it; only the writer behavior
//  changes). v1 still emits `recording.segment_closed` on stop so callers'
//  flow is unchanged when rotation arrives.
//

import Foundation

@available(macOS 13.0, *)
final class AudioRecorder {
    static let shared = AudioRecorder()

    private let queue = DispatchQueue(label: "corivo.helper.recorder", qos: .userInteractive)
    private var active: ActiveSession?

    struct StartRequest {
        var sessionID: String
        var outputDir: URL
        var segmentSeconds: Double
        var captureSystemAudio: Bool
        var captureMicrophone: Bool
        var microphoneDeviceID: String?
    }

    struct StartResponse {
        var sessionID: String
        var startedAt: Date
    }

    struct StopResponse {
        var sessionID: String
        var stoppedAt: Date
        var totalSegments: Int
    }

    func start(_ req: StartRequest) async throws -> StartResponse {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<StartResponse, Error>) in
            queue.async {
                if self.active != nil {
                    cont.resume(throwing: HelperError.resourceBusy(
                        "another recording session is already active"))
                    return
                }
                do {
                    try FileManager.default.createDirectory(
                        at: req.outputDir, withIntermediateDirectories: true)
                    let session = try ActiveSession(
                        request: req,
                        onSegmentClosed: { closed in
                            Task.detached {
                                let json: [String: Any] = [
                                    "session_id": closed.sessionID,
                                    "segment_index": closed.index,
                                    "path": closed.path.path,
                                    "duration_ms": UInt64(closed.duration * 1000),
                                    "started_at": Heartbeat.iso8601(closed.startedAt),
                                    "ended_at": Heartbeat.iso8601(closed.endedAt),
                                    "sys_track_present": closed.sysTrack,
                                    "mic_track_present": closed.micTrack,
                                ]
                                let evt = EventMessage(
                                    type: "event",
                                    name: "recording.segment_closed",
                                    ts: Heartbeat.iso8601(Date()),
                                    payload: AnyCodable(json)
                                )
                                try? await EventEmitter.shared.send(evt)
                            }
                        }
                    )
                    self.active = session
                    cont.resume(returning: StartResponse(
                        sessionID: req.sessionID,
                        startedAt: session.startedAt
                    ))
                } catch {
                    cont.resume(throwing: error)
                }
            }
        }
    }

    func stop(sessionID: String) async throws -> StopResponse {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<StopResponse, Error>) in
            queue.async {
                guard let session = self.active else {
                    cont.resume(throwing: HelperError.notFound(
                        "no active recording session"))
                    return
                }
                if session.sessionID != sessionID {
                    cont.resume(throwing: HelperError.notFound(
                        "session id mismatch: active=\(session.sessionID), requested=\(sessionID)"))
                    return
                }
                Task {
                    let total = await session.stop()
                    self.queue.async {
                        self.active = nil
                        cont.resume(returning: StopResponse(
                            sessionID: sessionID,
                            stoppedAt: Date(),
                            totalSegments: total
                        ))
                    }
                }
            }
        }
    }

    /// Emit a synthetic `recording.error` event. Used by sources to flag
    /// non-fatal issues (backpressure, brief ASBD mismatch) without
    /// terminating the session.
    static func emitWarning(sessionID: String, code: String, message: String) {
        Task.detached {
            let json: [String: Any] = [
                "session_id": sessionID,
                "code": code,
                "message": message,
                "fatal": false,
            ]
            let evt = EventMessage(
                type: "event",
                name: "recording.error",
                ts: Heartbeat.iso8601(Date()),
                payload: AnyCodable(json)
            )
            try? await EventEmitter.shared.send(evt)
        }
    }
}
