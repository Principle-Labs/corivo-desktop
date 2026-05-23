// control_handlers.hpp
// v1 control-plane handlers (ping / shutdown). Domain handlers live in
// per-feature files added in their respective phases.

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::control {

/// `ping` → `{ "pong": true }`. Liveness sanity check beyond heartbeat.
void ping(const corivo::proto::Request& request);

/// `shutdown` → `{ "ack": true }`, then exit cleanly with code 0.
void shutdown(const corivo::proto::Request& request);

} // namespace corivo::control
