//
//  ScreenHandlers.swift
//  RPC handlers for `screen.capture` and `screen.list_displays`.
//

import Foundation

enum ScreenHandlers {

    private struct CapturePayload: Decodable {
        var displayId: String?
        var outputPath: String?
        var format: String
        var quality: Int?

        enum CodingKeys: String, CodingKey {
            case displayId = "display_id"
            case outputPath = "output_path"
            case format
            case quality
        }
    }

    static func capture(_ request: RequestMessage) async throws {
        guard #available(macOS 14.0, *) else {
            throw HelperError.unsupported("screen.capture requires macOS 14.0 (SCScreenshotManager)")
        }
        let payload = try decodePayload(request.payload, as: CapturePayload.self)
        guard let format = ScreenCapture.Format(rawValue: payload.format) else {
            throw HelperError.invalidRequest("unknown format: \(payload.format)")
        }
        let req = ScreenCapture.Request(
            displayId: payload.displayId,
            outputPath: payload.outputPath,
            format: format,
            quality: payload.quality
        )
        let result = try await ScreenCapture.capture(req)
        let resultJson: [String: Any] = [
            "path": result.path,
            "width": result.width,
            "height": result.height,
            "captured_at": result.capturedAt,
            "display_id": result.displayId,
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(resultJson))
        )
    }

    static func listDisplays(_ request: RequestMessage) async throws {
        guard #available(macOS 14.0, *) else {
            throw HelperError.unsupported(
                "screen.list_displays requires macOS 14.0 (SCShareableContent)")
        }
        let displays = try await DisplayEnumerator.list()
        let displaysArray: [[String: Any]] = displays.map { d in
            [
                "id": d.id,
                "name": d.name,
                "width": d.width,
                "height": d.height,
                "is_main": d.isMain,
            ]
        }
        let resultJson: [String: Any] = ["displays": displaysArray]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(resultJson))
        )
    }

    /// Helper: decode `RequestMessage.payload` (an `AnyCodable`) into the
    /// strongly-typed payload struct. Throws `invalidRequest` on shape
    /// mismatch.
    private static func decodePayload<T: Decodable>(
        _ payload: AnyCodable?,
        as _: T.Type
    ) throws -> T {
        guard let p = payload else {
            throw HelperError.invalidRequest("missing payload")
        }
        let data: Data
        do {
            data = try JSONCodec.encoder.encode(p)
        } catch {
            throw HelperError.invalidRequest("payload reserialize failed: \(error)")
        }
        do {
            return try JSONCodec.decoder.decode(T.self, from: data)
        } catch {
            throw HelperError.invalidRequest("payload decode failed: \(error)")
        }
    }
}
