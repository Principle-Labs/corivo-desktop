// protocol.cpp

#include "protocol.hpp"

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

#include "../util/logger.hpp"

namespace corivo::proto {

Capabilities Capabilities::current() {
    Capabilities c;
    // Phase 1 — screen capture via Windows.Graphics.Capture (Win10 1903+).
    // We can't easily probe at hello time (would require a one-shot WGC
    // session); the WGC handler does the capability assertion. Build host
    // is required to be Win10 1903+ per CMakeLists.txt _WIN32_WINNT, so
    // the runtime check is mostly belt-and-suspenders.
    c.screen_capture = true;
    // Phase 2 — UI Automation. UIA is shipped on every modern Windows;
    // no per-bundle permission to gate on. Failures are reported per
    // call as OS_ERROR.
    c.ax_query = true;
    // Phase 3 — foreground monitor (SetWinEventHook) + AX subscription
    // bridge (focused_window_changed / title_changed via WinEvents; see
    // ax/ax_event_bridge.cpp for the v1-vs-spec divergence note).
    c.ax_events = true;
    c.foreground_monitor = true;
    // Phase 4 — Windows.Media.Ocr. Real recognition requires a Windows
    // language pack with OCR; the engine factory falls back to user
    // profile defaults if no requested language is installed.
    c.ocr_local = true;
    // Phase 5 — recording: pipeline still TODO on Windows. Helper
    // refuses recording.start with UNSUPPORTED until v2.
    c.audio_record = false;
    return c;
}

PlatformInfo current_platform() {
    PlatformInfo p;
    p.os = "windows";

    // Build version. RtlGetVersion is the modern accurate path
    // (GetVersionEx returns a shimmed value on un-manifested binaries).
    using RtlGetVersionFn = LONG(WINAPI*)(PRTL_OSVERSIONINFOW);
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    RTL_OSVERSIONINFOW v{};
    v.dwOSVersionInfoSize = sizeof(v);
    if (ntdll) {
        auto fn = reinterpret_cast<RtlGetVersionFn>(
            GetProcAddress(ntdll, "RtlGetVersion"));
        if (fn) { fn(&v); }
    }
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%lu.%lu.%lu",
                  v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber);
    p.os_version = buf;

    SYSTEM_INFO si{};
    GetNativeSystemInfo(&si);
    switch (si.wProcessorArchitecture) {
        case PROCESSOR_ARCHITECTURE_AMD64: p.arch = "x86_64"; break;
        case PROCESSOR_ARCHITECTURE_ARM64: p.arch = "arm64";  break;
        default:                           p.arch = "x86_64"; break;
    }
    return p;
}

void to_json(nlohmann::json& j, const LogMessage& m) {
    j = nlohmann::json{{"type", m.type}, {"level", m.level}, {"message", m.message}};
    if (m.target) j["target"] = *m.target;
}
void from_json(const nlohmann::json& j, LogMessage& m) {
    m.type = j.value("type", "log");
    m.level = j.value("level", "info");
    m.message = j.value("message", "");
    if (j.contains("target") && !j["target"].is_null()) {
        m.target = j["target"].get<std::string>();
    }
}

void to_json(nlohmann::json& j, const Request& r) {
    j = nlohmann::json{{"type", r.type}, {"id", r.id}, {"method", r.method}};
    if (!r.payload.is_null()) j["payload"] = r.payload;
}
void from_json(const nlohmann::json& j, Request& r) {
    r.type = j.value("type", "request");
    r.id = j.value("id", "");
    r.method = j.value("method", "");
    r.payload = j.contains("payload") ? j["payload"] : nlohmann::json(nullptr);
}

void to_json(nlohmann::json& j, const ResponseError& e) {
    j = nlohmann::json{{"code", e.code}, {"message", e.message}};
    if (!e.detail.is_null()) j["detail"] = e.detail;
}
void from_json(const nlohmann::json& j, ResponseError& e) {
    e.code = j.value("code", error_codes::INTERNAL);
    e.message = j.value("message", "");
    e.detail = j.contains("detail") ? j["detail"] : nlohmann::json(nullptr);
}

void to_json(nlohmann::json& j, const Response& r) {
    j = nlohmann::json{{"type", r.type}, {"id", r.id}, {"ok", r.ok}};
    if (r.ok) {
        j["result"] = r.result;
    } else if (r.error) {
        j["error"] = *r.error;
    }
}
void from_json(const nlohmann::json& j, Response& r) {
    r.type = j.value("type", "response");
    r.id = j.value("id", "");
    r.ok = j.value("ok", true);
    r.result = j.contains("result") ? j["result"] : nlohmann::json(nullptr);
    if (j.contains("error") && !j["error"].is_null()) {
        ResponseError e;
        from_json(j["error"], e);
        r.error = e;
    }
}

std::optional<Inbound> parse_inbound(const std::string& line) {
    nlohmann::json j;
    try {
        j = nlohmann::json::parse(line);
    } catch (const std::exception& e) {
        log::warn(std::string("parse_inbound: malformed JSON: ") + e.what());
        return std::nullopt;
    }
    auto type_it = j.find("type");
    if (type_it == j.end() || !type_it->is_string()) {
        log::warn("parse_inbound: missing 'type' field");
        return std::nullopt;
    }
    const std::string type = type_it->get<std::string>();
    try {
        if (type == "hello_ack") {
            HelloAck ack;
            from_json(j, ack);
            return Inbound{ack};
        }
        if (type == "request") {
            Request req;
            from_json(j, req);
            return Inbound{req};
        }
    } catch (const std::exception& e) {
        log::warn(std::string("parse_inbound: decode failed: ") + e.what());
        return std::nullopt;
    }
    log::warn(std::string("parse_inbound: unexpected type: ") + type);
    return std::nullopt;
}

Hello make_hello(const std::string& helper_version) {
    Hello h;
    h.type = "hello";
    h.helper_version = helper_version;
    h.supported_protocols = {"v1"};
    h.capabilities = Capabilities::current();
    h.platform = current_platform();
    return h;
}

} // namespace corivo::proto
