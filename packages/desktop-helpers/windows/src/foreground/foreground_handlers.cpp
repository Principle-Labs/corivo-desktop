// foreground_handlers.cpp

#include "foreground_handlers.hpp"

#include "foreground_monitor.hpp"
#include "../ipc/event_emitter.hpp"

namespace corivo::foreground {

namespace {

void respond_ok(const std::string& id, const nlohmann::json& result) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

void respond_err(const std::string& id, const char* code, const std::string& msg) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = false;
    resp.error = corivo::proto::ResponseError{code, msg, nullptr};
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace

void handle_subscribe(const corivo::proto::Request& request) {
    if (Monitor::instance().start()) {
        respond_ok(request.id, {{"subscribed", true}});
    } else {
        respond_err(request.id,
                    corivo::proto::error_codes::OS_ERROR,
                    "SetWinEventHook failed: " +
                    std::to_string(GetLastError()));
    }
}

void handle_unsubscribe(const corivo::proto::Request& request) {
    Monitor::instance().stop();
    respond_ok(request.id, {{"ack", true}});
}

void handle_current(const corivo::proto::Request& request) {
    auto cur = Monitor::instance().snapshot_current();
    nlohmann::json result = {
        {"pid", cur.pid},
        {"bundle_id", cur.bundle_id},
        {"app_name", cur.app_name},
        {"window_title", cur.window_title},
    };
    respond_ok(request.id, result);
}

} // namespace corivo::foreground
