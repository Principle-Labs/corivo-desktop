// ax_subscribe_handlers.hpp
// Phase 3 — `ax.subscribe` / `ax.unsubscribe` placeholder. Windows wiring
// (UIA event handler on a dedicated STA worker) lives in the spec at
// `apps/desktop/docs/capture-helper-windows-spec.md` § 6.4. Until that
// lands the helper acks subscriptions but emits no events, mirroring the
// Capabilities flag (`ax_events: false`).

#pragma once

#include "../ipc/protocol.hpp"

namespace corivo::ax {

void handle_subscribe(const corivo::proto::Request& request);
void handle_unsubscribe(const corivo::proto::Request& request);

} // namespace corivo::ax
