// ax_event_bridge.cpp

#include "ax_event_bridge.hpp"

#include <atomic>
#include <filesystem>
#include <mutex>
#include <string>
#include <thread>

#include <nlohmann/json.hpp>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

#include "../ipc/event_emitter.hpp"
#include "../util/iso8601.hpp"
#include "../util/logger.hpp"

namespace corivo::ax {

namespace {

// Custom WM_USER message to break the GetMessage loop on shutdown.
constexpr UINT WM_AX_SHUTDOWN = WM_USER + 1;

std::atomic<int>   g_active_pid{0};
std::atomic<bool>  g_running{false};
std::atomic<DWORD> g_pump_thread_id{0};
std::thread        g_pump_thread;

// State accessed by the WinEvent callback (single thread — the pump) and
// the subscribe/unsubscribe entry points (any thread). Mutex guards both.
std::mutex   g_state_mu;
HWND         g_last_window_hwnd = nullptr;
std::string  g_last_window_title;

std::string utf16_to_utf8(const wchar_t* w, int wlen) {
    if (!w || wlen <= 0) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, w, wlen, nullptr, 0,
                                   nullptr, nullptr);
    std::string out;
    out.resize(len);
    WideCharToMultiByte(CP_UTF8, 0, w, wlen, out.data(), len, nullptr, nullptr);
    return out;
}

std::string exe_basename_for_pid(DWORD pid) {
    if (!pid) return {};
    HANDLE proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!proc) return {};
    wchar_t path[MAX_PATH] = {};
    DWORD n = MAX_PATH;
    BOOL ok = QueryFullProcessImageNameW(proc, 0, path, &n);
    CloseHandle(proc);
    if (!ok) return {};
    std::filesystem::path p{std::wstring(path, n)};
    return utf16_to_utf8(p.filename().c_str(),
                         static_cast<int>(p.filename().native().size()));
}

std::string window_title(HWND hwnd) {
    if (!hwnd) return {};
    wchar_t buf[512] = {};
    int n = GetWindowTextW(hwnd, buf, 512);
    return utf16_to_utf8(buf, n);
}

void emit_event(const char* name, int pid,
                const std::string& bundle_id,
                const std::string& title) {
    nlohmann::json payload = nlohmann::json::object();
    payload["pid"] = pid;
    if (!bundle_id.empty()) payload["bundle_id"] = bundle_id;
    if (!title.empty())     payload["window_title"] = title;

    nlohmann::json wire = {
        {"type", "event"},
        {"name", name},
        {"ts", corivo::iso8601::now_utc()},
        {"payload", payload},
    };
    try {
        corivo::ipc::EventEmitter::instance().send_raw(wire);
    } catch (const std::exception& e) {
        log::warn(std::string("ax event emit failed: ") + e.what());
    }
}

void CALLBACK win_event_proc(HWINEVENTHOOK, DWORD event, HWND hwnd,
                              LONG idObject, LONG idChild,
                              DWORD, DWORD) {
    int active_pid = g_active_pid.load(std::memory_order_relaxed);
    if (active_pid == 0 || !hwnd) return;

    // Both events we care about apply only to top-level window objects.
    if (idObject != OBJID_WINDOW || idChild != CHILDID_SELF) return;

    DWORD pid = 0;
    GetWindowThreadProcessId(hwnd, &pid);
    if (static_cast<int>(pid) != active_pid) return;

    if (event == EVENT_SYSTEM_FOREGROUND) {
        HWND root = GetAncestor(hwnd, GA_ROOT);
        if (!root) root = hwnd;
        std::string title = window_title(root);
        {
            std::lock_guard<std::mutex> lk(g_state_mu);
            if (root == g_last_window_hwnd) return;
            g_last_window_hwnd = root;
            g_last_window_title = title;
        }
        emit_event("ax.focused_window_changed", static_cast<int>(pid),
                   exe_basename_for_pid(pid), title);
    } else if (event == EVENT_OBJECT_NAMECHANGE) {
        // Only count titles changes on top-level windows. If GetAncestor
        // disagrees with the reported hwnd, the rename is on some inner
        // container we don't care about.
        if (GetAncestor(hwnd, GA_ROOT) != hwnd) return;
        std::string title = window_title(hwnd);
        {
            std::lock_guard<std::mutex> lk(g_state_mu);
            if (title == g_last_window_title) return;
            g_last_window_title = title;
        }
        emit_event("ax.title_changed", static_cast<int>(pid),
                   exe_basename_for_pid(pid), title);
    }
}

void pump_thread_main() {
    g_pump_thread_id.store(GetCurrentThreadId(), std::memory_order_release);

    // OUT_OF_CONTEXT: callback runs on this thread (the registering one)
    // SKIPOWNPROCESS: don't fire for events from this helper process
    HWINEVENTHOOK h_focus = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
        nullptr, &win_event_proc, 0, 0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);

    HWINEVENTHOOK h_name = SetWinEventHook(
        EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE,
        nullptr, &win_event_proc, 0, 0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);

    if (!h_focus || !h_name) {
        log::warn("ax_event_bridge: SetWinEventHook returned null; events disabled");
    }

    MSG msg;
    while (g_running.load(std::memory_order_acquire)) {
        BOOL r = GetMessageW(&msg, nullptr, 0, 0);
        if (r == 0 || r == -1) break;
        if (msg.message == WM_AX_SHUTDOWN) break;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    if (h_focus) UnhookWinEvent(h_focus);
    if (h_name)  UnhookWinEvent(h_name);
}

void ensure_started() {
    bool expected = false;
    if (!g_running.compare_exchange_strong(expected, true)) return;
    g_pump_thread = std::thread(pump_thread_main);
}

} // namespace

void subscribe_pid(int pid) {
    ensure_started();
    g_active_pid.store(pid, std::memory_order_relaxed);
    std::lock_guard<std::mutex> lk(g_state_mu);
    g_last_window_hwnd = nullptr;
    g_last_window_title.clear();
}

void unsubscribe_pid() {
    g_active_pid.store(0, std::memory_order_relaxed);
    std::lock_guard<std::mutex> lk(g_state_mu);
    g_last_window_hwnd = nullptr;
    g_last_window_title.clear();
}

std::vector<int> active_subscriptions() {
    int pid = g_active_pid.load(std::memory_order_relaxed);
    if (pid == 0) return {};
    return {pid};
}

void shutdown_event_bridge() {
    if (!g_running.exchange(false)) return;
    DWORD tid = g_pump_thread_id.load(std::memory_order_acquire);
    if (tid) PostThreadMessageW(tid, WM_AX_SHUTDOWN, 0, 0);
    if (g_pump_thread.joinable()) g_pump_thread.join();
}

} // namespace corivo::ax
