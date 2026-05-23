// event_emitter.cpp

#include "event_emitter.hpp"

#include "../util/logger.hpp"

namespace corivo::ipc {

EventEmitter& EventEmitter::instance() {
    static EventEmitter inst;
    return inst;
}

EventEmitter::EventEmitter()
    : stdout_handle_(GetStdHandle(STD_OUTPUT_HANDLE)) {}

void EventEmitter::send_raw(const nlohmann::json& payload) {
    send_json(payload);
}

void EventEmitter::send_json(const nlohmann::json& payload) {
    std::string serialized = payload.dump();
    serialized.push_back('\n');

    std::lock_guard<std::mutex> lk(mu_);
    DWORD remaining = static_cast<DWORD>(serialized.size());
    const char* cursor = serialized.data();
    while (remaining > 0) {
        DWORD written = 0;
        BOOL ok = WriteFile(stdout_handle_, cursor, remaining, &written, nullptr);
        if (!ok || written == 0) {
            DWORD err = GetLastError();
            log::error("WriteFile to stdout failed: " + std::to_string(err));
            throw EmitError("WriteFile failed: " + std::to_string(err));
        }
        cursor += written;
        remaining -= written;
    }
}

} // namespace corivo::ipc
