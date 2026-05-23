// screen_handlers.cpp

#include "screen_handlers.hpp"
#include "screen_capture.hpp"
#include "display_enumerator.hpp"
#include "../ipc/event_emitter.hpp"
#include "../util/iso8601.hpp"
#include "../util/logger.hpp"

#include <stdexcept>

namespace corivo::screen {

void handle_capture(const corivo::proto::Request& request) {
    using corivo::proto::Response;
    using corivo::proto::ResponseError;
    using corivo::proto::error_codes::INVALID_REQUEST;
    using corivo::proto::error_codes::OS_ERROR;

    auto& payload = request.payload;
    if (!payload.is_object()) {
        Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{
            INVALID_REQUEST,
            "screen.capture: payload must be an object",
            nullptr,
        };
        corivo::ipc::EventEmitter::instance().send(resp);
        return;
    }

    CaptureRequest req;
    if (auto it = payload.find("display_id");
        it != payload.end() && it->is_string()) {
        req.display_id = it->get<std::string>();
    }
    if (auto it = payload.find("output_path");
        it != payload.end() && it->is_string()) {
        req.output_path = it->get<std::string>();
    }
    if (auto it = payload.find("quality");
        it != payload.end() && it->is_number_integer()) {
        req.quality = it->get<int>();
    }
    auto fmt_it = payload.find("format");
    if (fmt_it == payload.end() || !fmt_it->is_string()) {
        Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{
            INVALID_REQUEST,
            "screen.capture: missing 'format' (jpeg|png)",
            nullptr,
        };
        corivo::ipc::EventEmitter::instance().send(resp);
        return;
    }
    const std::string fmt = fmt_it->get<std::string>();
    if (fmt == "jpeg") {
        req.format = ImageFormat::Jpeg;
    } else if (fmt == "png") {
        req.format = ImageFormat::Png;
    } else {
        Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{
            INVALID_REQUEST,
            "screen.capture: unknown format: " + fmt,
            nullptr,
        };
        corivo::ipc::EventEmitter::instance().send(resp);
        return;
    }

    try {
        CaptureResponse out = capture(req);
        nlohmann::json result = {
            {"path", out.path},
            {"width", out.width},
            {"height", out.height},
            {"captured_at", corivo::iso8601::format(out.captured_at)},
            {"display_id", out.display_id},
        };
        Response resp;
        resp.id = request.id;
        resp.ok = true;
        resp.result = result;
        corivo::ipc::EventEmitter::instance().send(resp);
    } catch (const std::exception& e) {
        Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{OS_ERROR, e.what(), nullptr};
        corivo::ipc::EventEmitter::instance().send(resp);
    }
}

void handle_list_displays(const corivo::proto::Request& request) {
    using corivo::proto::Response;
    using corivo::proto::ResponseError;
    using corivo::proto::error_codes::OS_ERROR;

    try {
        auto displays = list_displays();
        nlohmann::json arr = nlohmann::json::array();
        for (auto const& d : displays) {
            arr.push_back({
                {"id", d.id},
                {"name", d.name},
                {"width", d.width},
                {"height", d.height},
                {"is_main", d.is_main},
            });
        }
        nlohmann::json result = {{"displays", arr}};
        Response resp;
        resp.id = request.id;
        resp.ok = true;
        resp.result = result;
        corivo::ipc::EventEmitter::instance().send(resp);
    } catch (const std::exception& e) {
        Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{OS_ERROR, e.what(), nullptr};
        corivo::ipc::EventEmitter::instance().send(resp);
    }
}

} // namespace corivo::screen
