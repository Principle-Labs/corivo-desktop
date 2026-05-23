// ax_subscribe_handlers.cpp

#include "ax_subscribe_handlers.hpp"

#include <string>
#include <vector>

#include "ax_event_bridge.hpp"
#include "../ipc/event_emitter.hpp"
#include "../util/logger.hpp"

namespace corivo::ax {

namespace {

void respond_error(const std::string& id, const char* code,
                   const std::string& msg) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = false;
    resp.error = corivo::proto::ResponseError{code, msg, nullptr};
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace

void handle_subscribe(const corivo::proto::Request& request) {
    using namespace corivo::proto;

    auto& payload = request.payload;
    if (!payload.is_object()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.subscribe: payload must be an object");
        return;
    }
    auto pid_it = payload.find("pid");
    if (pid_it == payload.end() || !pid_it->is_number_integer()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.subscribe: missing 'pid' integer");
        return;
    }
    int pid = pid_it->get<int>();

    // The `notifications` filter is accepted but currently advisory — v1
    // always emits both focused_window_changed and title_changed. The
    // response echoes the set we actually emit so the client can detect
    // a mismatch.
    std::vector<std::string> emitted = {"focused_window", "title"};

    subscribe_pid(pid);

    nlohmann::json result = {{"subscribed", emitted}};
    Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

void handle_unsubscribe(const corivo::proto::Request& request) {
    using namespace corivo::proto;

    // pid is informational — v1 only tracks a single subscription, so an
    // unsubscribe clears it regardless of which pid the client names.
    auto& payload = request.payload;
    if (payload.is_object()) {
        if (auto it = payload.find("pid");
            it != payload.end() && !it->is_number_integer()) {
            respond_error(request.id, error_codes::INVALID_REQUEST,
                          "ax.unsubscribe: 'pid' must be an integer");
            return;
        }
    }

    unsubscribe_pid();

    nlohmann::json result = {{"ack", true}};
    Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace corivo::ax
