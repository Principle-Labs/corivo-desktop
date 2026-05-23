//
//  OcrHandlers.swift
//  RPC handler for `ocr.run`.
//

import Foundation

enum OcrHandlers {

    private struct Payload: Decodable {
        var imagePath: String
        var languages: [String]?
        var useLanguageCorrection: Bool?

        enum CodingKeys: String, CodingKey {
            case imagePath = "image_path"
            case languages
            case useLanguageCorrection = "use_language_correction"
        }
    }

    static func run(_ request: RequestMessage) async throws {
        let payload = try decodePayload(request.payload, as: Payload.self)
        let req = VisionOCR.Request(
            imagePath: payload.imagePath,
            languages: payload.languages ?? ["zh-Hans", "zh-Hant", "en-US"],
            useLanguageCorrection: payload.useLanguageCorrection ?? false
        )
        let result = try await Task.detached(priority: .userInitiated) {
            try VisionOCR.run(req)
        }.value
        let json: [String: Any] = [
            "text": result.text,
            "elapsed_ms": result.elapsedMs,
        ]
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
