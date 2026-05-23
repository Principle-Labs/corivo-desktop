// ax_handlers.hpp
// RPC handlers for `ax.query` and `ax.probe_selection`.

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::ax {

void handle_query(const corivo::proto::Request& request);
void handle_probe_selection(const corivo::proto::Request& request);

} // namespace corivo::ax
