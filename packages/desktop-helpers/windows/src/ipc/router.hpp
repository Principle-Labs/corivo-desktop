// router.hpp
// Top-level dispatcher for inbound RPC. Phase 0 only handles control-plane
// methods (`ping` / `shutdown`). Domain methods are added in their own
// phases by extending the switch in router.cpp.

#pragma once

#include "protocol.hpp"

namespace corivo::ipc {

class Router {
public:
    /// Synchronous dispatch — runs the handler on the calling thread.
    /// main.cpp spawns a detached std::thread per request so the dispatch
    /// loop can stay responsive.
    static void dispatch(const corivo::proto::Request& request);
};

} // namespace corivo::ipc
