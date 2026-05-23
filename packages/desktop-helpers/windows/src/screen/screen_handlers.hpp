// screen_handlers.hpp
// RPC handlers for `screen.capture` and `screen.list_displays`.

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::screen {

void handle_capture(const corivo::proto::Request& request);
void handle_list_displays(const corivo::proto::Request& request);

} // namespace corivo::screen
