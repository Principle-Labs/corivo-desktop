// display_enumerator.hpp
// Lists monitors known to the OS via EnumDisplayMonitors. The id we hand
// out is the HMONITOR pointer rendered as decimal — opaque-but-stable
// for the lifetime of the helper process.

#pragma once

#include <string>
#include <vector>

namespace corivo::screen {

struct DisplayInfo {
    std::string id;
    std::string name;
    int width;
    int height;
    bool is_main;
};

/// Enumerate active monitors. Throws on EnumDisplayMonitors failure
/// (which essentially never happens on a healthy system).
std::vector<DisplayInfo> list_displays();

/// Resolve a monitor by id (the string returned in DisplayInfo::id), or
/// return the primary monitor when id is empty.
struct DisplayHandle {
    void* hmonitor;     // HMONITOR; stored as void* to avoid leaking <Windows.h> here
    int width;
    int height;
};

/// Throws std::runtime_error if the id doesn't resolve to a live monitor.
DisplayHandle resolve(const std::string& id_or_empty);

} // namespace corivo::screen
