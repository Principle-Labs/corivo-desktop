// heartbeat.cpp

#include "heartbeat.hpp"

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <string>
#include <thread>

#include "event_emitter.hpp"
#include "protocol.hpp"
#include "../ax/ax_event_bridge.hpp"
#include "../util/iso8601.hpp"
#include "../util/logger.hpp"

namespace corivo::ipc {

namespace {
std::atomic<bool> g_started{false};
constexpr unsigned long long DEFAULT_INTERVAL_MS = 5000;
} // namespace

void Heartbeat::start() {
    bool expected = false;
    if (!g_started.compare_exchange_strong(expected, true)) {
        return;  // already started; idempotent
    }

    const unsigned long long interval_ms = []() {
        auto override_val = parse_interval_override_ms();
        return override_val ? override_val : DEFAULT_INTERVAL_MS;
    }();

    std::thread([interval_ms]() {
        while (true) {
            std::this_thread::sleep_for(std::chrono::milliseconds(interval_ms));
            corivo::proto::Heartbeat hb;
            hb.ts = corivo::iso8601::now_utc();
            hb.ax_subscriptions = corivo::ax::active_subscriptions();
            try {
                EventEmitter::instance().send(hb);
            } catch (const std::exception& e) {
                log::warn(std::string("heartbeat write failed: ") + e.what());
                // Loop continues — next tick may succeed.
            }
        }
    }).detach();
}

unsigned long long Heartbeat::parse_interval_override_ms() {
    const char* raw = std::getenv("CORIVO_HELPER_HEARTBEAT_INTERVAL_MS");
    if (!raw || *raw == '\0') return 0;
    try {
        unsigned long long v = std::stoull(raw);
        return v > 0 ? v : 0;
    } catch (...) {
        return 0;
    }
}

} // namespace corivo::ipc
