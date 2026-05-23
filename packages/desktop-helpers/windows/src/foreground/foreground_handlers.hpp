// foreground_handlers.hpp

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::foreground {

void handle_subscribe(const corivo::proto::Request& request);
void handle_unsubscribe(const corivo::proto::Request& request);
void handle_current(const corivo::proto::Request& request);

} // namespace corivo::foreground
