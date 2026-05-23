//
//  Protocol.swift
//  Wire types for the capture-helper v1 protocol (Swift side mirror of
//  apps/desktop/src-tauri/src/services/capture_client/protocol.rs).
//
//  Each top-level message is its own struct that includes the `type`
//  discriminator as a constant — that lets us encode them through the
//  default JSONEncoder without any custom logic. For decoding we read the
//  `type` field via `MessageType` first, then dispatch to the right struct.
//

import Foundation

// MARK: - Common

enum HelperOS: String, Codable {
    case macos
    case windows
}

enum HelperArch: String, Codable {
    case arm64
    case x86_64 = "x86_64"
}

struct PlatformInfo: Codable {
    let os: HelperOS
    let osVersion: String
    let arch: HelperArch

    enum CodingKeys: String, CodingKey {
        case os
        case osVersion = "os_version"
        case arch
    }

    static func current() -> PlatformInfo {
        let info = ProcessInfo.processInfo.operatingSystemVersion
        let osVersion = "\(info.majorVersion).\(info.minorVersion).\(info.patchVersion)"
        #if arch(arm64)
        let arch: HelperArch = .arm64
        #else
        let arch: HelperArch = .x86_64
        #endif
        return PlatformInfo(os: .macos, osVersion: osVersion, arch: arch)
    }
}

struct Capabilities: Codable {
    var audioRecord: Bool = false
    var audioPerApp: Bool = false
    var screenCapture: Bool = false
    var axQuery: Bool = false
    var axEvents: Bool = false
    var foregroundMonitor: Bool = false
    var ocrLocal: Bool = false

    enum CodingKeys: String, CodingKey {
        case audioRecord = "audio_record"
        case audioPerApp = "audio_per_app"
        case screenCapture = "screen_capture"
        case axQuery = "ax_query"
        case axEvents = "ax_events"
        case foregroundMonitor = "foreground_monitor"
        case ocrLocal = "ocr_local"
    }

    /// Capabilities the current helper build supports. Each new phase
    /// flips a flag as it ships; OS-version-conditional features check
    /// availability at startup.
    static func current() -> Capabilities {
        var caps = Capabilities()
        // Phase 1: single-frame screen capture via ScreenCaptureKit.
        // SCScreenshotManager is the simple API (one call → CGImage); it
        // landed in macOS 14.0. Older 13.x falls back to SCStream + single
        // frame, which we'll add when there's actual demand.
        if #available(macOS 14.0, *) {
            caps.screenCapture = true
        }
        // Phase 2: AX text query + selection probe. The runtime
        // permission check (AXIsProcessTrusted) happens per-call; we
        // advertise capability unconditionally on macOS 13+ since the
        // API itself is always present.
        caps.axQuery = true
        // Phase 3 + Phase 7: foreground monitor + AX events (AXObserver
        // on a dedicated CFRunLoop thread, see AXObserverThread).
        caps.axEvents = true
        caps.foregroundMonitor = true
        // Phase 4: Vision OCR (always available 13+).
        caps.ocrLocal = true
        // Phase 5: meeting audio recording. Requires SCK system-audio
        // (13.0+) + AVCaptureSession microphone, both unconditionally
        // available on the supported platform range.
        caps.audioRecord = true
        return caps
    }
}

// MARK: - Message envelopes

/// Helper-emitted on startup (must be the first message).
struct HelloMessage: Codable {
    let type: String
    let helperVersion: String
    let supportedProtocols: [String]
    let capabilities: Capabilities
    let platform: PlatformInfo

    enum CodingKeys: String, CodingKey {
        case type
        case helperVersion = "helper_version"
        case supportedProtocols = "supported_protocols"
        case capabilities
        case platform
    }

    init(
        helperVersion: String,
        supportedProtocols: [String],
        capabilities: Capabilities,
        platform: PlatformInfo
    ) {
        self.type = "hello"
        self.helperVersion = helperVersion
        self.supportedProtocols = supportedProtocols
        self.capabilities = capabilities
        self.platform = platform
    }
}

/// Client-emitted in reply to hello (must arrive within 5s).
struct HelloAckMessage: Codable {
    let type: String
    let selectedProtocol: String
    let clientVersion: String

    enum CodingKeys: String, CodingKey {
        case type
        case selectedProtocol = "selected_protocol"
        case clientVersion = "client_version"
    }
}

/// Helper-emitted every 5s (or `CORIVO_HELPER_HEARTBEAT_INTERVAL_MS` if set).
struct HeartbeatMessage: Codable {
    let type: String
    let ts: String
    let inFlightRequests: Int
    let recordingSessions: [RecordingSessionStatus]
    let axSubscriptions: [Int]

    enum CodingKeys: String, CodingKey {
        case type
        case ts
        case inFlightRequests = "in_flight_requests"
        case recordingSessions = "recording_sessions"
        case axSubscriptions = "ax_subscriptions"
    }

    init(
        ts: String,
        inFlightRequests: Int = 0,
        recordingSessions: [RecordingSessionStatus] = [],
        axSubscriptions: [Int] = []
    ) {
        self.type = "heartbeat"
        self.ts = ts
        self.inFlightRequests = inFlightRequests
        self.recordingSessions = recordingSessions
        self.axSubscriptions = axSubscriptions
    }
}

struct RecordingSessionStatus: Codable {
    let sessionId: String
    let segmentsWritten: Int

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case segmentsWritten = "segments_written"
    }
}

/// Optional structured log over the main channel (rare; prefer stderr for
/// normal logs).
struct LogMessage: Codable {
    let type: String
    let level: String
    let message: String
    let target: String?

    enum CodingKeys: String, CodingKey {
        case type
        case level
        case message
        case target
    }

    init(level: String, message: String, target: String? = nil) {
        self.type = "log"
        self.level = level
        self.message = message
        self.target = target
    }
}

/// Client-emitted RPC.
struct RequestMessage: Codable {
    let type: String
    let id: String
    let method: String
    let payload: AnyCodable?
}

/// Helper-emitted reply to a Request.
struct ResponseMessage: Codable {
    let type: String
    let id: String
    let ok: Bool
    let result: AnyCodable?
    let error: ResponseError?

    init(id: String, result: AnyCodable) {
        self.type = "response"
        self.id = id
        self.ok = true
        self.result = result
        self.error = nil
    }

    init(id: String, error: ResponseError) {
        self.type = "response"
        self.id = id
        self.ok = false
        self.result = nil
        self.error = error
    }
}

struct ResponseError: Codable {
    let code: String
    let message: String
    let detail: AnyCodable?

    init(code: String, message: String, detail: AnyCodable? = nil) {
        self.code = code
        self.message = message
        self.detail = detail
    }
}

/// Helper-emitted async event (no domain events in Phase 0).
struct EventMessage: Codable {
    let type: String
    let name: String
    let ts: String
    let payload: AnyCodable?
}

/// v1 method names. New phases append; existing methods cannot change
/// semantics (only add optional fields).
enum Methods {
    // Phase 0 — control plane.
    static let ping = "ping"
    static let shutdown = "shutdown"

    // Phase 1 — single-frame screen capture.
    static let screenCapture = "screen.capture"
    static let screenListDisplays = "screen.list_displays"

    // Phase 2 — accessibility text + selection.
    static let axQuery = "ax.query"
    static let axProbeSelection = "ax.probe_selection"

    // Phase 3 — async subscriptions.
    static let axSubscribe = "ax.subscribe"
    static let axUnsubscribe = "ax.unsubscribe"
    static let foregroundSubscribe = "foreground.subscribe"
    static let foregroundUnsubscribe = "foreground.unsubscribe"
    static let foregroundCurrent = "foreground.current"

    // Phase 4 — local OCR.
    static let ocrRun = "ocr.run"

    // Phase 5 — meeting audio recording.
    static let recordingStart = "recording.start"
    static let recordingStop = "recording.stop"
    static let recordingListMicrophones = "recording.list_microphones"

    // Phase 7 — permission status.
    static let permissionStatus = "permission.status"
}

/// Standard error codes (mirror of v1 schema's `ErrorCode`).
enum ErrorCodes {
    static let invalidRequest = "INVALID_REQUEST"
    static let unsupported = "UNSUPPORTED"
    static let permissionDenied = "PERMISSION_DENIED"
    static let resourceBusy = "RESOURCE_BUSY"
    static let notFound = "NOT_FOUND"
    static let timeout = "TIMEOUT"
    static let `internal` = "INTERNAL"
    static let osError = "OS_ERROR"
}

/// Type-discriminator peek used by the dispatcher to route an inbound JSON
/// line to the correct typed decoder.
struct MessageType: Decodable {
    let type: String
}

/// What the helper expects to receive from the client.
enum InboundMessage {
    case helloAck(HelloAckMessage)
    case request(RequestMessage)
}

extension InboundMessage {
    static func decode(from data: Data) throws -> InboundMessage {
        let header = try JSONCodec.decoder.decode(MessageType.self, from: data)
        switch header.type {
        case "hello_ack":
            return .helloAck(try JSONCodec.decoder.decode(HelloAckMessage.self, from: data))
        case "request":
            return .request(try JSONCodec.decoder.decode(RequestMessage.self, from: data))
        default:
            throw HelperError.protocolViolation("unexpected inbound message type: \(header.type)")
        }
    }
}
