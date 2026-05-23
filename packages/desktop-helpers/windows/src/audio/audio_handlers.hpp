// audio_handlers.hpp
// Phase 5 — recording RPC handlers (`recording.start`, `recording.stop`,
// `recording.list_microphones`).
//
// **Status**: Windows recording is currently stubbed: handlers ack
// `list_microphones` with an empty list and reject `start` / `stop` with
// `UNSUPPORTED`. The real WASAPI loopback + WASAPI capture + Media
// Foundation Sink Writer pipeline lives in
// `apps/desktop/docs/capture-helper-windows-spec.md` § 4.1-4.5 — that
// pseudocode transcribes to ~600 LOC of platform code that needs Windows
// hardware to validate. Phase 5 v2 brings this online.

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::audio {

void handle_start(const corivo::proto::Request& request);
void handle_stop(const corivo::proto::Request& request);
void handle_list_microphones(const corivo::proto::Request& request);

} // namespace corivo::audio
