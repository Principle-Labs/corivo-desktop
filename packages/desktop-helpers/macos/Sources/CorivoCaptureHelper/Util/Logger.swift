//
//  Logger.swift
//  Tiny stderr logger. Each line is forwarded verbatim by the Rust
//  capture_client into its `tracing` pipeline (target `capture_helper`),
//  so format consistency matters more than levels here.
//

import Foundation

enum Log {
    enum Level: String {
        case trace, debug, info, warn, error
    }

    private static let stderr = FileHandle.standardError
    private static let lock = NSLock()

    static func trace(_ msg: String) { write(.trace, msg) }
    static func debug(_ msg: String) { write(.debug, msg) }
    static func info(_ msg: String)  { write(.info,  msg) }
    static func warn(_ msg: String)  { write(.warn,  msg) }
    static func error(_ msg: String) { write(.error, msg) }

    private static func write(_ level: Level, _ msg: String) {
        let line = "[\(level.rawValue)] \(msg)\n"
        lock.lock()
        defer { lock.unlock() }
        if let data = line.data(using: .utf8) {
            try? stderr.write(contentsOf: data)
        }
    }
}
