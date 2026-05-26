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

// Custom WM_USER message to break the GetMessage loop on shutdown.
constexpr UINT WM_FG_SHUTDOWN = WM_USER + 1;

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

Monitor::~Monitor() {
    stop();
}

bool Monitor::start() {
    bool expected = false;
    if (!running_.compare_exchange_strong(expected, true)) {
        return true;  // already running
    }

    std::promise<bool> hook_ready;
    std::future<bool> hook_ready_fut = hook_ready.get_future();

    pump_thread_ = std::thread(&Monitor::pump_thread_main, this,
                                std::move(hook_ready));

    bool ok = hook_ready_fut.get();
    if (!ok) {
        // Hook registration failed; pump_thread_main returned on its own.
        if (pump_thread_.joinable()) pump_thread_.join();
        running_.store(false);
    }
    return ok;
}

void Monitor::stop() {
    if (!running_.exchange(false)) return;
    DWORD tid = pump_thread_id_.load(std::memory_order_acquire);
    if (tid) PostThreadMessageW(tid, WM_FG_SHUTDOWN, 0, 0);
    if (pump_thread_.joinable()) pump_thread_.join();
}

void Monitor::pump_thread_main(std::promise<bool> hook_ready) {
    pump_thread_id_.store(GetCurrentThreadId(), std::memory_order_release);

    // OUT_OF_CONTEXT: callback runs on this thread (the registering one),
    // so we need a message pump here. SKIPOWNPROCESS: don't fire for
    // events from this helper process.
    HWINEVENTHOOK h = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
        nullptr, &Monitor::win_event_proc,
        /* idProcess */ 0, /* idThread */ 0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);

    if (!h) {
        log::warn("foreground_monitor: SetWinEventHook failed err=" +
                  std::to_string(GetLastError()));
        pump_thread_id_.store(0, std::memory_order_release);
        hook_ready.set_value(false);
        return;
    }
    hook_ready.set_value(true);

    MSG msg;
    while (running_.load(std::memory_order_acquire)) {
        BOOL r = GetMessageW(&msg, nullptr, 0, 0);
        if (r == 0 || r == -1) break;
        if (msg.message == WM_FG_SHUTDOWN) break;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    UnhookWinEvent(h);
    pump_thread_id_.store(0, std::memory_order_release);
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
