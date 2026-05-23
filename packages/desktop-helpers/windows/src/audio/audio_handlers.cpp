// audio_handlers.cpp

#include "audio_handlers.hpp"

#include "../ipc/event_emitter.hpp"

namespace corivo::audio {

namespace {

void respond_unsupported(const std::string& id, const char* what) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = false;
    resp.error = corivo::proto::ResponseError{
        corivo::proto::error_codes::UNSUPPORTED,
        std::string(what) + ": Windows recording pipeline not yet implemented "
        "(see capture-helper-windows-spec.md § 4)",
        nullptr,
    };
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace

void handle_start(const corivo::proto::Request& request) {
    respond_unsupported(request.id, "recording.start");
}

void handle_stop(const corivo::proto::Request& request) {
    respond_unsupported(request.id, "recording.stop");
}

void handle_list_microphones(const corivo::proto::Request& request) {
    // List microphones doesn't strictly need the recording pipeline; it
    // can run via WASAPI MMDeviceEnumerator (cheap, no permissions). v2
    // will fill in the actual enumeration; for v1 we ack with an empty
    // list so the UI doesn't break.
    nlohmann::json result = {{"devices", nlohmann::json::array()}};
    corivo::proto::Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace corivo::audio
