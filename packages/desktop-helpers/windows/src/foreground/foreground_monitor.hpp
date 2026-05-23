// foreground_monitor.hpp
// SetWinEventHook(EVENT_SYSTEM_FOREGROUND) wrapper. WINEVENT_OUTOFCONTEXT
// means the callback fires on a system-managed thread with no need for a
// message pump in our process — a clean fit for a console sidecar.

#pragma once

#include <atomic>
#include <mutex>
#include <string>

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
    bool is_running() const { return hook_ != nullptr; }

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

    static void CALLBACK win_event_proc(HWINEVENTHOOK, DWORD event, HWND hwnd,
                                         LONG idObject, LONG idChild,
                                         DWORD eventThread, DWORD eventTime);

    void emit_for(HWND hwnd);

    std::atomic<HWINEVENTHOOK> hook_{nullptr};
    mutable std::mutex mu_;
    std::string last_bundle_;
};

} // namespace corivo::foreground
