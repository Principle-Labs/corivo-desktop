// display_enumerator.cpp

#include "display_enumerator.hpp"

#include <stdexcept>
#include <vector>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

namespace corivo::screen {

namespace {

struct Acc {
    std::vector<DisplayInfo> displays;
};

BOOL CALLBACK enum_proc(HMONITOR hmon, HDC, LPRECT, LPARAM lparam) {
    auto* acc = reinterpret_cast<Acc*>(lparam);
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    if (!GetMonitorInfoW(hmon, &info)) return TRUE;

    DisplayInfo d;
    char id[32];
    std::snprintf(id, sizeof(id), "%llu",
                  static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(hmon)));
    d.id = id;

    // Convert the device name (UTF-16) to UTF-8 for the wire.
    int wlen = static_cast<int>(wcslen(info.szDevice));
    int u8len = WideCharToMultiByte(CP_UTF8, 0, info.szDevice, wlen,
                                    nullptr, 0, nullptr, nullptr);
    std::string name;
    name.resize(u8len);
    WideCharToMultiByte(CP_UTF8, 0, info.szDevice, wlen,
                        name.data(), u8len, nullptr, nullptr);
    d.name = std::move(name);

    d.width  = info.rcMonitor.right  - info.rcMonitor.left;
    d.height = info.rcMonitor.bottom - info.rcMonitor.top;
    d.is_main = (info.dwFlags & MONITORINFOF_PRIMARY) != 0;

    acc->displays.push_back(std::move(d));
    return TRUE;
}

} // namespace

std::vector<DisplayInfo> list_displays() {
    Acc acc;
    if (!EnumDisplayMonitors(nullptr, nullptr, enum_proc,
                             reinterpret_cast<LPARAM>(&acc))) {
        throw std::runtime_error("EnumDisplayMonitors failed: " +
                                 std::to_string(GetLastError()));
    }
    return acc.displays;
}

DisplayHandle resolve(const std::string& id_or_empty) {
    HMONITOR target = nullptr;
    if (id_or_empty.empty()) {
        // Primary monitor.
        const POINT origin{0, 0};
        target = MonitorFromPoint(origin, MONITOR_DEFAULTTOPRIMARY);
    } else {
        unsigned long long raw = 0;
        try {
            raw = std::stoull(id_or_empty);
        } catch (...) {
            throw std::runtime_error("display id is not numeric: " + id_or_empty);
        }
        target = reinterpret_cast<HMONITOR>(static_cast<uintptr_t>(raw));
    }

    MONITORINFO info{};
    info.cbSize = sizeof(info);
    if (!GetMonitorInfoW(target, &info)) {
        throw std::runtime_error("display id not live: " + id_or_empty);
    }

    DisplayHandle h;
    h.hmonitor = target;
    h.width  = info.rcMonitor.right  - info.rcMonitor.left;
    h.height = info.rcMonitor.bottom - info.rcMonitor.top;
    return h;
}

} // namespace corivo::screen
