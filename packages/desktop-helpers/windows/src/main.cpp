// main.cpp
// Entry point for the Windows capture helper sidecar.
//
// Top-level flow (per architecture spec § 6):
//   1. Switch stdio to binary mode (preserve \r\n in JSON payloads).
//   2. Parent-pid guard      — refuse to run unless launched by Rust main.
//   3. Parse args            — `--echo-mode` for CI / protocol smoke.
//   4. Emit `hello`          — first message after spawn (must be < 5s).
//   5. Await `hello_ack`     — Rust client confirms protocol selection.
//   6. Start heartbeat       — periodic liveness emit.
//   7. Dispatch loop         — read NDJSON requests, route through Router.
//   8. EOF / shutdown        — exit cleanly.
//
// This file is the orchestration only; per-feature handlers live under
// control/ (Phase 0) and audio/ screen/ ax/ foreground/ ocr/ (later).

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <io.h>
#include <fcntl.h>
#include <thread>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

#include <winrt/base.h>

#include "ipc/codec.hpp"
#include "ipc/event_emitter.hpp"
#include "ipc/heartbeat.hpp"
#include "ipc/protocol.hpp"
#include "ipc/router.hpp"
#include "util/logger.hpp"

namespace {

constexpr const char* HELPER_VERSION = "0.1.0";

bool flag_set(const wchar_t* env_name) {
    wchar_t buf[4];
    DWORD n = GetEnvironmentVariableW(env_name, buf, 4);
    return n > 0 && n < 4 && buf[0] == L'1' && buf[1] == L'\0';
}

bool env_present(const wchar_t* env_name) {
    return GetEnvironmentVariableW(env_name, nullptr, 0) > 0;
}

} // namespace

int wmain(int argc, wchar_t** argv) {
    // 1. stdio → binary mode. Without this Windows would translate every
    //    \n we emit into \r\n on stdout, corrupting NDJSON line framing.
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);

    // 1.5. COM init — MTA. Required for WinRT (WGC), WIC, and most COM-
    //       backed APIs we'll touch in later phases.
    try {
        winrt::init_apartment(winrt::apartment_type::multi_threaded);
    } catch (winrt::hresult_error const& e) {
        std::fprintf(stderr,
                     "CorivoCaptureHelper: COM init failed: 0x%08lX\n",
                     static_cast<unsigned long>(e.code().value));
        return 8;
    }

    // 2. Parent-pid guard. CI / dev tooling can opt out via env.
    if (!flag_set(L"CORIVO_HELPER_SKIP_PARENT_PID")) {
        if (!env_present(L"CORIVO_HELPER_PARENT_PID")) {
            std::fputs(
                "CorivoCaptureHelper: refusing to start: missing CORIVO_HELPER_PARENT_PID\n",
                stderr);
            return 2;
        }
    }

    // 3. Parse args. `--echo-mode` reserved for future phases that need
    //    to skip platform setup when running in CI smoke mode.
    bool echo_mode = false;
    for (int i = 1; i < argc; ++i) {
        if (std::wcscmp(argv[i], L"--echo-mode") == 0) echo_mode = true;
    }
    corivo::log::info(echo_mode ? "starting in --echo-mode (protocol-only)"
                                : "starting in normal mode");

    // 4. Emit hello.
    {
        auto hello = corivo::proto::make_hello(HELPER_VERSION);
        try {
            corivo::ipc::EventEmitter::instance().send(hello);
        } catch (const std::exception& e) {
            corivo::log::error(std::string("failed to send hello: ") + e.what());
            return 3;
        }
    }

    // 5. Await hello_ack. No per-helper deadline: the Rust client has its
    //    own 5s handshake timeout and will SIGKILL us if we take too long,
    //    so blocking forever is safe.
    corivo::ipc::LineReader reader(GetStdHandle(STD_INPUT_HANDLE));
    {
        std::optional<std::string> first;
        try {
            first = reader.read_line();
        } catch (const std::exception& e) {
            corivo::log::error(std::string("hello_ack read failed: ") + e.what());
            return 6;
        }
        if (!first) {
            corivo::log::error("stdin EOF before hello_ack");
            return 4;
        }
        auto inbound = corivo::proto::parse_inbound(*first);
        if (!inbound) {
            corivo::log::error("first message was not a valid envelope");
            return 5;
        }
        if (auto* ack = std::get_if<corivo::proto::HelloAck>(&*inbound)) {
            corivo::log::info("hello_ack received protocol=" + ack->selected_protocol +
                              " client=" + ack->client_version);
        } else {
            corivo::log::error("expected hello_ack as first message from client");
            return 5;
        }
    }

    // 6. Start heartbeat. Phase 0 has nothing in-flight to report, so the
    //    payload is the bare timestamp.
    corivo::ipc::Heartbeat::start();

    // 7. Dispatch loop. Each request gets its own detached thread so the
    //    loop never stalls on a slow handler.
    while (true) {
        std::optional<std::string> line;
        try {
            line = reader.read_line();
        } catch (const std::exception& e) {
            corivo::log::error(std::string("dispatch read failed: ") + e.what());
            return 7;
        }
        if (!line) {
            corivo::log::info("stdin EOF, exiting cleanly");
            return 0;
        }
        if (line->empty()) continue;

        auto inbound = corivo::proto::parse_inbound(*line);
        if (!inbound) {
            corivo::log::warn("decode failed (line dropped)");
            continue;
        }
        if (auto* req = std::get_if<corivo::proto::Request>(&*inbound)) {
            // Capture by value so the thread owns a stable copy.
            corivo::proto::Request req_copy = *req;
            std::thread([req_copy]() {
                corivo::ipc::Router::dispatch(req_copy);
            }).detach();
        } else {
            corivo::log::warn("unexpected hello_ack post-handshake (ignored)");
        }
    }
}
