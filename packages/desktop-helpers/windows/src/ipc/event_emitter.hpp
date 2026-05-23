// event_emitter.hpp
// Thread-safe stdout writer. Every send is atomic at the line level so
// concurrent producers (handler threads, heartbeat thread) can never
// interleave bytes within a JSON line.

#pragma once

#include <mutex>
#include <stdexcept>
#include <string>

#include <nlohmann/json.hpp>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

namespace corivo::ipc {

class EmitError : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

class EventEmitter {
public:
    static EventEmitter& instance();

    /// Encode `message` to JSON, append `\n`, write atomically.
    template <typename T>
    void send(const T& message) {
        nlohmann::json j = message;
        send_json(j);
    }

    /// Bypass the typed encoder for messages whose shape can't be expressed
    /// by the typed structs (test hooks, future forward-compat scenarios).
    void send_raw(const nlohmann::json& payload);

private:
    EventEmitter();

    void send_json(const nlohmann::json& payload);

    std::mutex mu_;
    HANDLE stdout_handle_;
};

} // namespace corivo::ipc
