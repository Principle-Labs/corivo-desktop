// foreground_monitor.hpp
// SetWinEventHook(EVENT_SYSTEM_FOREGROUND) wrapper. WINEVENT_OUTOFCONTEXT
// delivers callbacks via the message queue of the thread that called
// SetWinEventHook, so we own a dedicated pump thread with a GetMessage
// loop — same pattern as ax_event_bridge.cpp.

#pragma once

#include <atomic>
#include <future>
#include <mutex>
#include <string>
#include <thread>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

namespace corivo::foreground {

class Monitor {
public:
    static Monitor& instance();

    bool start();
    void stop();
    bool is_running() const { return running_.load(std::memory_order_acquire); }

    /// Synchronous current foreground app — used by `foreground.current`.
    struct Current {
        int pid = 0;
        std::string bundle_id;
        std::string app_name;
        std::string window_title;
    };
    Current snapshot_current() const;

private:
    Monitor() = default;
    ~Monitor();

    void pump_thread_main(std::promise<bool> hook_ready);

    static void CALLBACK win_event_proc(HWINEVENTHOOK, DWORD event, HWND hwnd,
                                         LONG idObject, LONG idChild,
                                         DWORD eventThread, DWORD eventTime);

    void emit_for(HWND hwnd);

    std::atomic<bool>  running_{false};
    std::atomic<DWORD> pump_thread_id_{0};
    std::thread        pump_thread_;
    mutable std::mutex mu_;
    std::string        last_bundle_;
};

} // namespace corivo::foreground
