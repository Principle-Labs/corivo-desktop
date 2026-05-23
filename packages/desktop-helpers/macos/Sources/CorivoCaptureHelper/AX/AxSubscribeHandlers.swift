//
//  AxSubscribeHandlers.swift
//  RPC handlers for `ax.subscribe` / `ax.unsubscribe`. Wires the
//  request payload through to the dedicated [`AXObserverThread`] which
//  manages a single AXObserver with retargeting (one pid at a time, per
//  spec). Events fire from the AX callback on the observer thread and
//  go straight to the helper's event emitter.
//

import Foundation

enum AxSubscribeHandlers {

    private struct SubscribePayload: Decodable {
        var pid: pid_t
        var notifications: [String]
    }

    private struct UnsubscribePayload: Decodable {
        var pid: pid_t
    }

    static func subscribe(_ request: RequestMessage) async throws {
        let payload = try decodePayload(request.payload, as: SubscribePayload.self)
        let notifications = payload.notifications.compactMap(AxNotification.init(rawValue:))
        AXObserverThread.shared.subscribe(
            pid: payload.pid,
            notifications: notifications
        )
        let json: [String: Any] = [
            "subscribed": notifications.map { $0.rawValue },
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }

    static func unsubscribe(_ request: RequestMessage) async throws {
        let payload = try decodePayload(request.payload, as: UnsubscribePayload.self)
        AXObserverThread.shared.unsubscribe(pid: payload.pid)
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(["ack": true]))
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
