// ax_handlers.cpp
//
// ax.query and ax.probe_selection both route their UIA work through the
// AXExecutor pool (ax_executor.{hpp,cpp}). The dispatcher thread parses
// the request, submits the UIA call, and waits on the resulting future
// with an outer deadline (the request's own deadline + a small grace).
// On timeout the dispatcher returns TIMEOUT to the client immediately;
// the worker's task keeps running until it returns naturally (UIA has no
// cancellation primitive), but its result is dropped via shared_ptr
// release — never touched again.

#include "ax_handlers.hpp"

#include <chrono>
#include <exception>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>

#include "accessibility_query.hpp"
#include "ax_executor.hpp"
#include "selection_probe.hpp"
#include "../ipc/event_emitter.hpp"
#include "../util/logger.hpp"

namespace corivo::ax {

namespace {

constexpr auto OUTER_GRACE = std::chrono::milliseconds(50);

void respond_error(const std::string& id, const char* code,
                   const std::string& msg) {
    corivo::proto::Response resp;
    resp.id = id;
    resp.ok = false;
    resp.error = corivo::proto::ResponseError{code, msg, nullptr};
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace

void handle_query(const corivo::proto::Request& request) {
    using namespace corivo::proto;

    auto& payload = request.payload;
    if (!payload.is_object()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.query: payload must be an object");
        return;
    }
    auto pid_it = payload.find("pid");
    if (pid_it == payload.end() || !pid_it->is_number_integer()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.query: missing 'pid' integer");
        return;
    }

    QueryRequest req;
    req.pid = pid_it->get<int>();
    if (auto it = payload.find("max_depth");
        it != payload.end() && it->is_number_integer()) {
        req.max_depth = it->get<int>();
    }
    if (auto it = payload.find("max_chars");
        it != payload.end() && it->is_number_integer()) {
        req.max_chars = it->get<int>();
    }
    if (auto it = payload.find("deadline_ms");
        it != payload.end() && it->is_number_integer()) {
        req.deadline = std::chrono::milliseconds(it->get<int>());
    }
    if (auto skip_it = payload.find("skip_predicate");
        skip_it != payload.end() && skip_it->is_object()) {
        if (auto it = skip_it->find("skip_roles");
            it != skip_it->end() && it->is_array()) {
            for (auto const& v : *it) {
                if (v.is_string()) req.skip.skip_roles.insert(v.get<std::string>());
            }
        }
        if (auto it = skip_it->find("skip_subroles");
            it != skip_it->end() && it->is_array()) {
            for (auto const& v : *it) {
                if (v.is_string()) req.skip.skip_subroles.insert(v.get<std::string>());
            }
        }
        if (auto it = skip_it->find("skip_descriptions_substr");
            it != skip_it->end() && it->is_array()) {
            for (auto const& v : *it) {
                if (v.is_string()) req.skip.skip_descriptions_substr.push_back(v.get<std::string>());
            }
        }
    }

    struct Work {
        QueryRequest req;
        std::optional<QueryResponse> result;
        std::exception_ptr err;
    };
    auto work = std::make_shared<Work>();
    work->req = req;

    auto submitted = submit([work]() {
        try {
            work->result = query(work->req);
        } catch (...) {
            work->err = std::current_exception();
        }
    });

    if (submitted.status == SubmitStatus::Busy) {
        respond_error(request.id, error_codes::RESOURCE_BUSY,
                      "ax.query: executor queue full");
        return;
    }

    auto wait_budget = req.deadline + OUTER_GRACE;
    if (submitted.future.wait_for(wait_budget) == std::future_status::timeout) {
        respond_error(request.id, error_codes::TIMEOUT,
                      "ax.query exceeded deadline (" +
                          std::to_string(req.deadline.count()) + "ms)");
        return;
    }

    if (work->err) {
        try {
            std::rethrow_exception(work->err);
        } catch (const std::exception& e) {
            respond_error(request.id, error_codes::OS_ERROR, e.what());
            return;
        } catch (...) {
            respond_error(request.id, error_codes::OS_ERROR,
                          "ax.query: unknown exception");
            return;
        }
    }

    if (!work->result) {
        respond_error(request.id, error_codes::INTERNAL,
                      "ax.query: worker returned no result");
        return;
    }

    auto const& out = *work->result;
    nlohmann::json result = {
        {"text", out.text},
        {"elapsed_ms", out.elapsed_ms},
        {"truncated", out.truncated},
    };
    if (out.reason) {
        result["reason"] = *out.reason;
    }
    Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

void handle_probe_selection(const corivo::proto::Request& request) {
    using namespace corivo::proto;

    auto& payload = request.payload;
    if (!payload.is_object()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.probe_selection: payload must be an object");
        return;
    }
    auto pid_it = payload.find("pid");
    if (pid_it == payload.end() || !pid_it->is_number_integer()) {
        respond_error(request.id, error_codes::INVALID_REQUEST,
                      "ax.probe_selection: missing 'pid'");
        return;
    }
    int pid = pid_it->get<int>();
    auto deadline = std::chrono::milliseconds(250);
    if (auto it = payload.find("deadline_ms");
        it != payload.end() && it->is_number_integer()) {
        deadline = std::chrono::milliseconds(it->get<int>());
    }

    struct Work {
        int pid;
        std::chrono::milliseconds deadline;
        std::optional<std::string> selection;
        std::exception_ptr err;
    };
    auto work = std::make_shared<Work>();
    work->pid = pid;
    work->deadline = deadline;

    auto submitted = submit([work]() {
        try {
            work->selection = probe_selection(work->pid, work->deadline);
        } catch (...) {
            work->err = std::current_exception();
        }
    });

    if (submitted.status == SubmitStatus::Busy) {
        respond_error(request.id, error_codes::RESOURCE_BUSY,
                      "ax.probe_selection: executor queue full");
        return;
    }

    auto wait_budget = deadline + OUTER_GRACE;
    if (submitted.future.wait_for(wait_budget) == std::future_status::timeout) {
        respond_error(request.id, error_codes::TIMEOUT,
                      "ax.probe_selection exceeded deadline (" +
                          std::to_string(deadline.count()) + "ms)");
        return;
    }

    if (work->err) {
        try {
            std::rethrow_exception(work->err);
        } catch (const std::exception& e) {
            respond_error(request.id, error_codes::OS_ERROR, e.what());
            return;
        } catch (...) {
            respond_error(request.id, error_codes::OS_ERROR,
                          "ax.probe_selection: unknown exception");
            return;
        }
    }

    nlohmann::json result;
    if (work->selection) {
        result = {{"selection", *work->selection}};
    } else {
        result = {{"selection", nullptr}};
    }

    Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace corivo::ax
