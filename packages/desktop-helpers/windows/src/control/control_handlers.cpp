// control_handlers.cpp

#include "control_handlers.hpp"

#include <chrono>
#include <cstdlib>
#include <thread>

#include "../ipc/event_emitter.hpp"
#include "../util/logger.hpp"

namespace corivo::control {

void ping(const corivo::proto::Request& request) {
    corivo::proto::Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = nlohmann::json{{"pong", true}};
    corivo::ipc::EventEmitter::instance().send(resp);
}

void shutdown(const corivo::proto::Request& request) {
    corivo::proto::Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = nlohmann::json{{"ack", true}};
    corivo::ipc::EventEmitter::instance().send(resp);

    // Spec § 5.4: give writer time to flush before tearing down. Helper
    // shares stdout with no other producers at this moment, so a small
    // pause is enough.
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
    log::info("shutdown.ack_sent_exiting");
    std::exit(0);
}

} // namespace corivo::control
