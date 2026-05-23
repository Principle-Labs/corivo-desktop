//
//  RecordingHandlers.swift
//  RPC handlers for `recording.start`, `recording.stop`,
//  `recording.list_microphones`.
//

import Foundation

enum RecordingHandlers {

    private struct StartPayload: Decodable {
        var sessionId: String
        var outputDir: String
        var segmentSeconds: Double
        var captureSystemAudio: Bool
        var captureMicrophone: Bool
        var microphoneDeviceId: String?

        enum CodingKeys: String, CodingKey {
            case sessionId = "session_id"
            case outputDir = "output_dir"
            case segmentSeconds = "segment_seconds"
            case captureSystemAudio = "capture_system_audio"
            case captureMicrophone = "capture_microphone"
            case microphoneDeviceId = "microphone_device_id"
        }
    }

    private struct StopPayload: Decodable {
        var sessionId: String
        enum CodingKeys: String, CodingKey { case sessionId = "session_id" }
    }

    static func start(_ request: RequestMessage) async throws {
        guard #available(macOS 13.0, *) else {
            throw HelperError.unsupported("recording requires macOS 13.0+ (ScreenCaptureKit audio)")
        }
        let payload = try decodePayload(request.payload, as: StartPayload.self)
        let req = AudioRecorder.StartRequest(
            sessionID: payload.sessionId,
            outputDir: URL(fileURLWithPath: payload.outputDir),
            segmentSeconds: payload.segmentSeconds,
            captureSystemAudio: payload.captureSystemAudio,
            captureMicrophone: payload.captureMicrophone,
            microphoneDeviceID: payload.microphoneDeviceId
        )
        let result = try await AudioRecorder.shared.start(req)
        let json: [String: Any] = [
            "session_id": result.sessionID,
            "started_at": Heartbeat.iso8601(result.startedAt),
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    static func stop(_ request: RequestMessage) async throws {
        guard #available(macOS 13.0, *) else {
            throw HelperError.unsupported("recording requires macOS 13.0+")
        }
        let payload = try decodePayload(request.payload, as: StopPayload.self)
        let result = try await AudioRecorder.shared.stop(sessionID: payload.sessionId)
        let json: [String: Any] = [
            "session_id": result.sessionID,
            "stopped_at": Heartbeat.iso8601(result.stoppedAt),
            "total_segments": result.totalSegments,
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    static func listMicrophones(_ request: RequestMessage) async throws {
        guard #available(macOS 13.0, *) else {
            throw HelperError.unsupported("recording requires macOS 13.0+")
        }
        let devices = MicrophoneSource.enumerateDevices()
        let arr: [[String: Any]] = devices.map {
            ["id": $0.id, "name": $0.name, "is_default": $0.isDefault]
        }
        let json: [String: Any] = ["devices": arr]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    private static func decodePayload<T: Decodable>(
        _ payload: AnyCodable?, as _: T.Type
    ) throws -> T {
        guard let p = payload else {
            throw HelperError.invalidRequest("missing payload")
        }
        let data = try JSONCodec.encoder.encode(p)
        return try JSONCodec.decoder.decode(T.self, from: data)
    }
}
