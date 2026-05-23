//
//  Codec.swift
//  NDJSON line reader/writer + the shared JSONEncoder / JSONDecoder.
//
//  Reader implements a manual byte-by-byte loop over `FileHandle.bytes`
//  with a hard line-length cap so a misbehaving client can't OOM us.
//

import Foundation

/// Hard cap on a single NDJSON line. Mirrors the Rust client's
/// `MAX_LINE_BYTES`. 1 MiB fits any realistic AX walk output (max 64 KiB
/// in spec) plus envelope overhead with margin.
let MAX_LINE_BYTES: Int = 1024 * 1024

enum CodecError: Error {
    case eof
    case lineTooLong(Int)
    case invalidJSON(Error)
    case io(Error)
}

extension CodecError: CustomStringConvertible {
    var description: String {
        switch self {
        case .eof: return "eof"
        case .lineTooLong(let n): return "line exceeded \(n) bytes without newline"
        case .invalidJSON(let e): return "invalid JSON: \(e)"
        case .io(let e): return "io: \(e)"
        }
    }
}

enum JSONCodec {
    static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        // No outputFormatting — keep messages on a single line.
        return e
    }()

    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        return d
    }()
}

/// Async line stream over a FileHandle. Each `readLine()` call returns one
/// `\n`-terminated payload (with the newline stripped) or nil on EOF.
/// Throws on lines that exceed [`MAX_LINE_BYTES`].
///
/// **Concurrency contract**: only one task reads from a given LineStream
/// at a time. We don't use `actor` because `actor` isolation conflicts
/// with calling the mutating async `AsyncIterator.next()` (Swift 6 strict
/// concurrency rejects it: an `await` mid-iteration could let another
/// `readLine()` invocation observe a torn iterator state). Helper boot
/// sequence reads from a single dispatch loop, so single-reader is both
/// natural and enforced by structure.
final class LineStream: @unchecked Sendable {
    private var iterator: FileHandle.AsyncBytes.AsyncIterator
    private var buffer: [UInt8] = []

    init(_ handle: FileHandle) {
        self.iterator = handle.bytes.makeAsyncIterator()
    }

    func readLine(maxBytes: Int = MAX_LINE_BYTES) async throws -> Data? {
        while true {
            let next: UInt8?
            do {
                next = try await iterator.next()
            } catch {
                throw CodecError.io(error)
            }
            guard let byte = next else {
                if buffer.isEmpty { return nil }
                let last = Data(buffer)
                buffer.removeAll(keepingCapacity: true)
                return last
            }
            if byte == 0x0A {
                // Strip trailing \r if present (Windows pipes).
                if let last = buffer.last, last == 0x0D {
                    buffer.removeLast()
                }
                let line = Data(buffer)
                buffer.removeAll(keepingCapacity: true)
                return line
            }
            buffer.append(byte)
            if buffer.count > maxBytes {
                throw CodecError.lineTooLong(maxBytes)
            }
        }
    }
}
