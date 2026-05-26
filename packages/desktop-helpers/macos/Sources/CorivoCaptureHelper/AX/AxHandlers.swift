//
//  AxHandlers.swift
//  RPC handlers for `ax.query` and `ax.probe_selection`.
//
//  Wrap the synchronous AX calls in `Task.detached` so the helper's
//  dispatch loop stays responsive while a deep tree walks.
//

import Foundation

enum AxHandlers {

    private struct QueryPayload: Decodable {
        var pid: Int32
        var maxDepth: Int?
        var maxChars: Int?
        var deadlineMs: Int?
        var skipPredicate: SkipPayload?

        enum CodingKeys: String, CodingKey {
            case pid
            case maxDepth = "max_depth"
            case maxChars = "max_chars"
            case deadlineMs = "deadline_ms"
            case skipPredicate = "skip_predicate"
        }
    }

    private struct SkipPayload: Decodable {
        var skipRoles: [String]?
        var skipSubroles: [String]?
        var skipDescriptionsSubstr: [String]?

        enum CodingKeys: String, CodingKey {
            case skipRoles = "skip_roles"
            case skipSubroles = "skip_subroles"
            case skipDescriptionsSubstr = "skip_descriptions_substr"
        }
    }

    private struct ProbePayload: Decodable {
        var pid: Int32
        var deadlineMs: Int?

        enum CodingKeys: String, CodingKey {
            case pid
            case deadlineMs = "deadline_ms"
        }
    }

    static func query(_ request: RequestMessage) async throws {
        let payload = try decodePayload(request.payload, as: QueryPayload.self)
        var skip = AccessibilityQuery.SkipPredicate()
        if let s = payload.skipPredicate {
            if let v = s.skipRoles                  { skip.skipRoles = Set(v) }
            if let v = s.skipSubroles               { skip.skipSubroles = Set(v) }
            if let v = s.skipDescriptionsSubstr     { skip.skipDescriptionsSubstr = v }
        }
        let req = AccessibilityQuery.Request(
            pid: payload.pid,
            maxDepth: payload.maxDepth ?? 64,
            maxChars: payload.maxChars ?? 64_000,
            deadlineSeconds: TimeInterval(payload.deadlineMs ?? 1500) / 1000.0,
            skip: skip
        )

        // Detach to a background task so the dispatch loop isn't blocked
        // by the deep, deadline-bounded walk.
        let result = try await Task.detached(priority: .userInitiated) {
            try AccessibilityQuery.query(req)
        }.value

        var json: [String: Any] = [
            "text": result.text,
            "elapsed_ms": result.elapsedMs,
            "truncated": result.truncated,
        ]
        if let reason = result.reason {
            json["reason"] = reason
        }
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    static func probeSelection(_ request: RequestMessage) async throws {
        let payload = try decodePayload(request.payload, as: ProbePayload.self)
        let deadlineSec = TimeInterval(payload.deadlineMs ?? 250) / 1000.0
        let pid = payload.pid

        let result = try await Task.detached(priority: .userInitiated) {
            try SelectionProbe.probe(pid: pid, deadlineSeconds: deadlineSec)
        }.value

        let json: [String: Any] = [
            "selection": result as Any,
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    private static func decodePayload<T: Decodable>(
        _ payload: AnyCodable?,
        as _: T.Type
    ) throws -> T {
        guard let p = payload else {
            throw HelperError.invalidRequest("missing payload")
        }
        let data = try JSONCodec.encoder.encode(p)
        do {
            return try JSONCodec.decoder.decode(T.self, from: data)
        } catch {
            throw HelperError.invalidRequest("payload decode failed: \(error)")
        }
    }
}
