// router.cpp

#include "router.hpp"

#include <exception>
#include <string>

#include "event_emitter.hpp"
#include "../control/control_handlers.hpp"
#include "../screen/screen_handlers.hpp"
#include "../ax/ax_handlers.hpp"
#include "../ax/ax_subscribe_handlers.hpp"
#include "../foreground/foreground_handlers.hpp"
#include "../ocr/ocr_handlers.hpp"
#include "../audio/audio_handlers.hpp"
#include "../permissions/permission_handlers.hpp"
#include "../util/logger.hpp"

namespace corivo::ipc {

namespace {

void send_error(const std::string& id, const char* code, const std::string& message) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = false;
    corivo::proto::ResponseError err;
    err.code = code;
    err.message = message;
    resp.error = err;
    try {
        EventEmitter::instance().send(resp);
    } catch (const std::exception& e) {
        log::error(std::string("router: response write failed: ") + e.what());
    }
}

} // namespace

void Router::dispatch(const corivo::proto::Request& request) {
    try {
        if (request.method == corivo::proto::methods::PING) {
            corivo::control::ping(request);
        } else if (request.method == corivo::proto::methods::SHUTDOWN) {
            corivo::control::shutdown(request);
        } else if (request.method == corivo::proto::methods::SCREEN_CAPTURE) {
            corivo::screen::handle_capture(request);
        } else if (request.method == corivo::proto::methods::SCREEN_LIST_DISPLAYS) {
            corivo::screen::handle_list_displays(request);
        } else if (request.method == corivo::proto::methods::AX_QUERY) {
            corivo::ax::handle_query(request);
        } else if (request.method == corivo::proto::methods::AX_PROBE_SELECTION) {
            corivo::ax::handle_probe_selection(request);
        } else if (request.method == corivo::proto::methods::AX_SUBSCRIBE) {
            corivo::ax::handle_subscribe(request);
        } else if (request.method == corivo::proto::methods::AX_UNSUBSCRIBE) {
            corivo::ax::handle_unsubscribe(request);
        } else if (request.method == corivo::proto::methods::FOREGROUND_SUBSCRIBE) {
            corivo::foreground::handle_subscribe(request);
        } else if (request.method == corivo::proto::methods::FOREGROUND_UNSUBSCRIBE) {
            corivo::foreground::handle_unsubscribe(request);
        } else if (request.method == corivo::proto::methods::FOREGROUND_CURRENT) {
            corivo::foreground::handle_current(request);
        } else if (request.method == corivo::proto::methods::OCR_RUN) {
            corivo::ocr::handle_run(request);
        } else if (request.method == corivo::proto::methods::RECORDING_START) {
            corivo::audio::handle_start(request);
        } else if (request.method == corivo::proto::methods::RECORDING_STOP) {
            corivo::audio::handle_stop(request);
        } else if (request.method == corivo::proto::methods::RECORDING_LIST_MICROPHONES) {
            corivo::audio::handle_list_microphones(request);
        } else if (request.method == corivo::proto::methods::PERMISSION_STATUS) {
            corivo::permissions::handle_status(request);
        } else {
            send_error(request.id,
                       corivo::proto::error_codes::UNSUPPORTED,
                       "unknown method: " + request.method);
        }
    } catch (const std::exception& e) {
        send_error(request.id,
                   corivo::proto::error_codes::INTERNAL,
                   std::string("handler raised: ") + e.what());
    } catch (...) {
        send_error(request.id,
                   corivo::proto::error_codes::INTERNAL,
                   "handler raised non-std exception");
    }
}

} // namespace corivo::ipc
