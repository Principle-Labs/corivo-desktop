// selection_probe.hpp
// UIA TextPattern selection — Windows analog of macOS AXTextMarker.

#pragma once

#include <chrono>
#include <optional>
#include <string>

namespace corivo::ax {

/// Returns the user's currently-highlighted text in `pid`'s focused
/// element, or nullopt if there's no selection / element doesn't support
/// TextPattern / etc. (the common case). Never throws — failure modes
/// are reported via nullopt to keep the call cheap on the hot path.
std::optional<std::string> probe_selection(int pid,
                                            std::chrono::milliseconds deadline);

} // namespace corivo::ax
