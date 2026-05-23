// ocr_handlers.hpp

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::ocr {

void handle_run(const corivo::proto::Request& request);

} // namespace corivo::ocr
