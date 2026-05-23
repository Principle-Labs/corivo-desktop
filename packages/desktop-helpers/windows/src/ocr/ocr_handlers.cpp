// ocr_handlers.cpp

#include "ocr_handlers.hpp"

#include <stdexcept>

#include "windows_ocr.hpp"
#include "../ipc/event_emitter.hpp"

namespace corivo::ocr {

void handle_run(const corivo::proto::Request& request) {
    using namespace corivo::proto;

    auto& payload = request.payload;
    if (!payload.is_object()) {
        // Qualify explicitly: an unqualified `Response` resolves to
        // `corivo::ocr::Response` (the OCR result type defined in
        // windows_ocr.hpp) because we're inside `namespace corivo::ocr`,
        // and the `using namespace corivo::proto;` above doesn't beat
        // closer scope. We want the wire response here.
        corivo::proto::Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{
            error_codes::INVALID_REQUEST,
            "ocr.run: payload must be an object",
            nullptr,
        };
        corivo::ipc::EventEmitter::instance().send(resp);
        return;
    }

    Request req;
    if (auto it = payload.find("image_path");
        it != payload.end() && it->is_string()) {
        req.image_path = it->get<std::string>();
    }
    if (req.image_path.empty()) {
        // Qualify explicitly: an unqualified `Response` resolves to
        // `corivo::ocr::Response` (the OCR result type defined in
        // windows_ocr.hpp) because we're inside `namespace corivo::ocr`,
        // and the `using namespace corivo::proto;` above doesn't beat
        // closer scope. We want the wire response here.
        corivo::proto::Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{
            error_codes::INVALID_REQUEST,
            "ocr.run: missing 'image_path'",
            nullptr,
        };
        corivo::ipc::EventEmitter::instance().send(resp);
        return;
    }
    if (auto it = payload.find("languages");
        it != payload.end() && it->is_array()) {
        for (auto const& v : *it) {
            if (v.is_string()) req.languages.push_back(v.get<std::string>());
        }
    }
    if (req.languages.empty()) {
        req.languages = {"zh-Hans", "zh-Hant", "en-US"};
    }
    if (auto it = payload.find("use_language_correction");
        it != payload.end() && it->is_boolean()) {
        req.use_language_correction = it->get<bool>();
    }

    try {
        auto out = run(req);
        nlohmann::json result = {
            {"text", out.text},
            {"elapsed_ms", out.elapsed_ms},
        };
        // Qualify explicitly: an unqualified `Response` resolves to
        // `corivo::ocr::Response` (the OCR result type defined in
        // windows_ocr.hpp) because we're inside `namespace corivo::ocr`,
        // and the `using namespace corivo::proto;` above doesn't beat
        // closer scope. We want the wire response here.
        corivo::proto::Response resp;
        resp.id = request.id;
        resp.ok = true;
        resp.result = result;
        corivo::ipc::EventEmitter::instance().send(resp);
    } catch (const std::exception& e) {
        // Qualify explicitly: an unqualified `Response` resolves to
        // `corivo::ocr::Response` (the OCR result type defined in
        // windows_ocr.hpp) because we're inside `namespace corivo::ocr`,
        // and the `using namespace corivo::proto;` above doesn't beat
        // closer scope. We want the wire response here.
        corivo::proto::Response resp;
        resp.id = request.id;
        resp.ok = false;
        resp.error = ResponseError{error_codes::OS_ERROR, e.what(), nullptr};
        corivo::ipc::EventEmitter::instance().send(resp);
    }
}

} // namespace corivo::ocr
