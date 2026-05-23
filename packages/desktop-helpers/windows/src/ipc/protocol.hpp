// protocol.hpp
// Wire types for the capture-helper v1 protocol (Windows side mirror of
// apps/desktop/src-tauri/src/services/capture_client/protocol.rs and
// helpers/macos/.../IPC/Protocol.swift).
//
// Serialization uses nlohmann::json's intrusive macros where the struct
// shape is fixed. Variant-shaped fields (Request.payload, Response.result,
// Response.error.detail) carry `nlohmann::json` directly so they can be
// any JSON value without ad-hoc Any-codable plumbing on this side.

#pragma once

#include <nlohmann/json.hpp>

#include <optional>
#include <string>
#include <variant>
#include <vector>

namespace corivo::proto {

// MARK: - Common

struct PlatformInfo {
    std::string os;            // "windows"
    std::string os_version;    // e.g. "10.0.22631"
    std::string arch;          // "arm64" | "x86_64"
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(PlatformInfo, os, os_version, arch);

PlatformInfo current_platform();

struct Capabilities {
    bool audio_record       = false;
    bool audio_per_app      = false;
    bool screen_capture     = false;
    bool ax_query           = false;
    bool ax_events          = false;
    bool foreground_monitor = false;
    bool ocr_local          = false;

    /// Capabilities the current helper build supports. Each phase flips
    /// the corresponding flag as it ships. Runtime version checks (e.g.
    /// WGC requires Win10 1903+) happen in the per-feature handler.
    static Capabilities current();
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(Capabilities,
                               audio_record,
                               audio_per_app,
                               screen_capture,
                               ax_query,
                               ax_events,
                               foreground_monitor,
                               ocr_local);

// MARK: - Message envelopes

struct Hello {
    std::string type = "hello";
    std::string helper_version;
    std::vector<std::string> supported_protocols;
    Capabilities capabilities;
    PlatformInfo platform;
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(Hello,
                               type,
                               helper_version,
                               supported_protocols,
                               capabilities,
                               platform);

struct HelloAck {
    std::string type;
    std::string selected_protocol;
    std::string client_version;
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(HelloAck, type, selected_protocol, client_version);

struct RecordingSessionStatus {
    std::string session_id;
    int segments_written = 0;
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(RecordingSessionStatus, session_id, segments_written);

struct Heartbeat {
    std::string type = "heartbeat";
    std::string ts;            // ISO8601 RFC3339 with millis
    int in_flight_requests = 0;
    std::vector<RecordingSessionStatus> recording_sessions;
    std::vector<int> ax_subscriptions;
};
NLOHMANN_DEFINE_TYPE_NON_INTRUSIVE(Heartbeat,
                               type,
                               ts,
                               in_flight_requests,
                               recording_sessions,
                               ax_subscriptions);

struct LogMessage {
    std::string type = "log";
    std::string level;
    std::string message;
    std::optional<std::string> target;
};
// Note: optional fields require explicit (de)serialization since the
// intrusive macro doesn't handle std::optional out of the box.
void to_json(nlohmann::json& j, const LogMessage& m);
void from_json(const nlohmann::json& j, LogMessage& m);

struct Request {
    std::string type;          // "request"
    std::string id;            // UUID
    std::string method;
    nlohmann::json payload = nullptr;  // free-form
};
void to_json(nlohmann::json& j, const Request& r);
void from_json(const nlohmann::json& j, Request& r);

struct ResponseError {
    std::string code;
    std::string message;
    nlohmann::json detail = nullptr;
};
void to_json(nlohmann::json& j, const ResponseError& e);
void from_json(const nlohmann::json& j, ResponseError& e);

struct Response {
    std::string type = "response";
    std::string id;
    bool ok = true;
    nlohmann::json result = nullptr;
    std::optional<ResponseError> error;
};
void to_json(nlohmann::json& j, const Response& r);
void from_json(const nlohmann::json& j, Response& r);

// MARK: - Methods / error codes (mirror of v1 schema enums)

namespace methods {
// Phase 0 — control plane.
inline constexpr const char* PING                 = "ping";
inline constexpr const char* SHUTDOWN             = "shutdown";

// Phase 1 — screen.
inline constexpr const char* SCREEN_CAPTURE       = "screen.capture";
inline constexpr const char* SCREEN_LIST_DISPLAYS = "screen.list_displays";

// Phase 2 — accessibility.
inline constexpr const char* AX_QUERY             = "ax.query";
inline constexpr const char* AX_PROBE_SELECTION   = "ax.probe_selection";

// Phase 3 — async subscriptions.
inline constexpr const char* AX_SUBSCRIBE         = "ax.subscribe";
inline constexpr const char* AX_UNSUBSCRIBE       = "ax.unsubscribe";
inline constexpr const char* FOREGROUND_SUBSCRIBE = "foreground.subscribe";
inline constexpr const char* FOREGROUND_UNSUBSCRIBE = "foreground.unsubscribe";
inline constexpr const char* FOREGROUND_CURRENT   = "foreground.current";

// Phase 4 — local OCR.
inline constexpr const char* OCR_RUN              = "ocr.run";

// Phase 5 — meeting audio recording.
inline constexpr const char* RECORDING_START               = "recording.start";
inline constexpr const char* RECORDING_STOP                = "recording.stop";
inline constexpr const char* RECORDING_LIST_MICROPHONES    = "recording.list_microphones";

// Phase 7 — system permission status.
inline constexpr const char* PERMISSION_STATUS             = "permission.status";
} // namespace methods

namespace error_codes {
inline constexpr const char* INVALID_REQUEST   = "INVALID_REQUEST";
inline constexpr const char* UNSUPPORTED       = "UNSUPPORTED";
inline constexpr const char* PERMISSION_DENIED = "PERMISSION_DENIED";
inline constexpr const char* RESOURCE_BUSY     = "RESOURCE_BUSY";
inline constexpr const char* NOT_FOUND         = "NOT_FOUND";
inline constexpr const char* TIMEOUT           = "TIMEOUT";
inline constexpr const char* INTERNAL          = "INTERNAL";
inline constexpr const char* OS_ERROR          = "OS_ERROR";
} // namespace error_codes

// MARK: - Inbound message dispatch

/// What the helper expects to receive from the client.
using Inbound = std::variant<HelloAck, Request>;

/// Parse an inbound NDJSON line. Returns nullopt for unknown / malformed
/// messages — caller logs and drops the line.
std::optional<Inbound> parse_inbound(const std::string& line);

/// Build a Phase 0 `hello` for this helper.
Hello make_hello(const std::string& helper_version);

} // namespace corivo::proto
