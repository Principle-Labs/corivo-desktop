// permission_handlers.hpp
// Phase 7 — `permission.status` handler. Mirrors the macOS handler's
// shape; on Windows UIA needs no permission, screen capture needs no
// permission, and microphone is governed by Settings > Privacy >
// Microphone (queried via Windows.Devices.Enumeration).

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::permissions {

void handle_status(const corivo::proto::Request& request);

} // namespace corivo::permissions
