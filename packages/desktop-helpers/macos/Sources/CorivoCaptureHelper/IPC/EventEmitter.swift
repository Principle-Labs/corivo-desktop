//
//  EventEmitter.swift
//  Serializes all stdout writes so concurrent producers (handler tasks,
//  heartbeat task, etc.) can never interleave bytes within a JSON line.
//
//  Implemented as an `actor`. Every send is awaited on the actor's serial
//  executor → atomic line writes without explicit locks.
//

import Foundation

actor EventEmitter {
    static let shared = EventEmitter()

    private let stdout: FileHandle = .standardOutput

    private init() {}

    /// Encode `message` to JSON, append `\n`, write atomically. Errors are
    /// logged to stderr and propagated.
    func send<T: Encodable>(_ message: T) throws {
        var data = try JSONCodec.encoder.encode(message)
        data.append(0x0A)
        do {
            try stdout.write(contentsOf: data)
        } catch {
            Log.error("stdout write failed: \(error)")
            throw CodecError.io(error)
        }
    }

    /// Bypass the typed encoder for cases where the message must contain
    /// values that the typed structs can't represent (test hooks, future
    /// forward-compat scenarios). Pass an already-validated JSON object.
    func sendRaw(_ jsonObject: [String: Any]) throws {
        guard JSONSerialization.isValidJSONObject(jsonObject) else {
            throw CodecError.invalidJSON(NSError(
                domain: "EventEmitter",
                code: -1,
                userInfo: [NSLocalizedDescriptionKey: "not a valid JSON object"]
            ))
        }
        var data = try JSONSerialization.data(withJSONObject: jsonObject, options: [])
        data.append(0x0A)
        do {
            try stdout.write(contentsOf: data)
        } catch {
            Log.error("stdout raw write failed: \(error)")
            throw CodecError.io(error)
        }
    }
}
