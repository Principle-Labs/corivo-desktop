// heartbeat.hpp
// Periodic liveness emit. Spec § 5.4 default cadence: every 5 seconds.
//
// Cadence can be shortened via `CORIVO_HELPER_HEARTBEAT_INTERVAL_MS`
// for tests that need to exercise health-monitor transitions in seconds
// rather than 15-second windows.

#pragma once

namespace corivo::ipc {

class Heartbeat {
public:
    /// Spawn a detached worker thread that emits one heartbeat every
    /// interval ms. Idempotent: subsequent calls are ignored.
    static void start();

private:
    static unsigned long long parse_interval_override_ms();
};

} // namespace corivo::ipc
