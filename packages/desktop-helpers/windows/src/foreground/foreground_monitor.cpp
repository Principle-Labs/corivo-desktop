// foreground_monitor.cpp

#include "foreground_monitor.hpp"

#include <cstdio>
#include <filesystem>
#include <string>

#include <nlohmann/json.hpp>

#include "../ipc/event_emitter.hpp"
#include "../util/iso8601.hpp"
#include "../util/logger.hpp"

namespace corivo::foreground {

namespace {

std::string utf16_to_utf8(const wchar_t* w, int wlen) {
    if (!w || wlen == 0) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, w, wlen, nullptr, 0,
                                   nullptr, nullptr);
    std::string out;
    out.resize(len);
    WideCharToMultiByte(CP_UTF8, 0, w, wlen, out.data(), len,
                        nullptr, nullptr);
    return out;
}

std::string exe_basename_for(HWND hwnd) {
    DWORD pid = 0;
    GetWindowThreadProcessId(hwnd, &pid);
    if (!pid) return {};
    HANDLE proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!proc) return {};
    wchar_t path[MAX_PATH] = {};
    DWORD path_len = MAX_PATH;
    BOOL ok = QueryFullProcessImageNameW(proc, 0, path, &path_len);
    CloseHandle(proc);
    if (!ok) return {};
    std::filesystem::path p{std::wstring(path, path_len)};
    return utf16_to_utf8(p.filename().c_str(),
                         static_cast<int>(p.filename().native().size()));
}

std::string window_title_for(HWND hwnd) {
    wchar_t title[256] = {};
    int n = GetWindowTextW(hwnd, title, 256);
    return utf16_to_utf8(title, n);
}

DWORD pid_for(HWND hwnd) {
    DWORD pid = 0;
    GetWindowThreadProcessId(hwnd, &pid);
    return pid;
}

} // namespace

Monitor& Monitor::instance() {
    static Monitor m;
    return m;
}

bool Monitor::start() {
    if (hook_.load() != nullptr) return true;
    HWINEVENTHOOK h = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
        nullptr, &Monitor::win_event_proc,
        /* idProcess */ 0, /* idThread */ 0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
    if (!h) return false;
    hook_.store(h);
    return true;
}

void Monitor::stop() {
    HWINEVENTHOOK h = hook_.exchange(nullptr);
    if (h) UnhookWinEvent(h);
}

void Monitor::win_event_proc(HWINEVENTHOOK, DWORD event, HWND hwnd,
                              LONG idObject, LONG idChild,
                              DWORD, DWORD) {
    if (event != EVENT_SYSTEM_FOREGROUND) return;
    if (idObject != OBJID_WINDOW || idChild != CHILDID_SELF) return;
    instance().emit_for(hwnd);
}

void Monitor::emit_for(HWND hwnd) {
    std::string bundle_id = exe_basename_for(hwnd);

    {
        std::lock_guard<std::mutex> lk(mu_);
        if (bundle_id == last_bundle_) return;
        last_bundle_ = bundle_id;
    }

    nlohmann::json payload = {
        {"pid", static_cast<int>(pid_for(hwnd))},
        {"bundle_id", bundle_id},
        {"app_name", bundle_id},
        {"window_title", window_title_for(hwnd)},
    };
    // The Windows protocol.hpp doesn't yet define a typed EventMessage
    // (events were introduced in Phase 3 alongside this code); send via
    // the raw-JSON escape hatch on EventEmitter.
    nlohmann::json wire = {
        {"type", "event"},
        {"name", "foreground.app_activated"},
        {"ts", corivo::iso8601::now_utc()},
        {"payload", payload},
    };
    try {
        corivo::ipc::EventEmitter::instance().send_raw(wire);
    } catch (const std::exception& e) {
        log::warn(std::string("foreground emit failed: ") + e.what());
    }
}

Monitor::Current Monitor::snapshot_current() const {
    HWND hwnd = GetForegroundWindow();
    Current c;
    if (!hwnd) return c;
    c.pid = static_cast<int>(pid_for(hwnd));
    c.bundle_id = exe_basename_for(hwnd);
    c.app_name = c.bundle_id;
    c.window_title = window_title_for(hwnd);
    return c;
}

} // namespace corivo::foreground
