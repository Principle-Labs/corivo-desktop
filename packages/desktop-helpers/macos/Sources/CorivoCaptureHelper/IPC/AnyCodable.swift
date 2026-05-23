//
//  AnyCodable.swift
//  Minimal Any-codable wrapper for the protocol's free-form payload /
//  result / error.detail fields.
//
//  Constraints we need to satisfy:
//  - Round-trip arbitrary JSON values (null / bool / number / string /
//    array / object) without information loss within the JSON type system
//  - Encode/decode through `JSONEncoder` / `JSONDecoder`
//  - Never crash on hostile input (JSON layer is the trust boundary)
//

import Foundation

struct AnyCodable: Codable {
    let value: Any

    init(_ value: Any) {
        self.value = value
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self.value = NSNull()
        } else if let v = try? container.decode(Bool.self) {
            self.value = v
        } else if let v = try? container.decode(Int64.self) {
            self.value = v
        } else if let v = try? container.decode(Double.self) {
            self.value = v
        } else if let v = try? container.decode(String.self) {
            self.value = v
        } else if let v = try? container.decode([AnyCodable].self) {
            self.value = v.map { $0.value }
        } else if let v = try? container.decode([String: AnyCodable].self) {
            self.value = v.mapValues { $0.value }
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "AnyCodable: unsupported JSON value"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch value {
        case is NSNull:
            try container.encodeNil()
        case let v as Bool:
            try container.encode(v)

        // Signed-integer family. Every width is listed explicitly because
        // Swift's `as Int` cast fails for sibling widths (Int32 / Int64
        // etc.) when the runtime type is concrete instead of NSNumber-
        // bridged. Promote everything to Int64.
        case let v as Int:
            try container.encode(Int64(v))
        case let v as Int8:
            try container.encode(Int64(v))
        case let v as Int16:
            try container.encode(Int64(v))
        case let v as Int32:
            try container.encode(Int64(v))
        case let v as Int64:
            try container.encode(v)

        // Unsigned-integer family. UInt64 is the case that bit us first:
        // elapsed_ms / duration_ms in the OCR / AX / recording responses
        // are all `UInt64`, and the previous default branch threw
        // `unsupported value of type UInt64`. Values up to Int64.max
        // round-trip losslessly; anything above that drops to Double
        // with a stderr warning so it's visible in logs.
        case let v as UInt:
            try container.encode(Int64(v))
        case let v as UInt8:
            try container.encode(Int64(v))
        case let v as UInt16:
            try container.encode(Int64(v))
        case let v as UInt32:
            try container.encode(Int64(v))
        case let v as UInt64:
            if v <= UInt64(Int64.max) {
                try container.encode(Int64(v))
            } else {
                FileHandle.standardError.write(Data(
                    "[warn] AnyCodable: UInt64 \(v) exceeds Int64.max; encoding as Double (lossy)\n"
                        .utf8
                ))
                try container.encode(Double(v))
            }

        case let v as Float:
            try container.encode(Double(v))
        case let v as Double:
            try container.encode(v)
        case let v as String:
            try container.encode(v)
        case let v as Date:
            // Encode dates as RFC3339-with-millis to match Rust's
            // chrono::DateTime<Utc> wire shape. Most call sites already
            // pre-format via `Heartbeat.iso8601`; this branch catches
            // stragglers that stuff a `Date` directly into the dict
            // (some recording-event handlers do).
            try container.encode(Heartbeat.iso8601(v))
        case let v as [Any]:
            try container.encode(v.map { AnyCodable($0) })
        case let v as [String: Any]:
            try container.encode(v.mapValues { AnyCodable($0) })
        default:
            throw EncodingError.invalidValue(
                value,
                EncodingError.Context(
                    codingPath: encoder.codingPath,
                    debugDescription: "AnyCodable: unsupported value of type \(type(of: value))"
                )
            )
        }
    }
}
