// ax_event_bridge.hpp
//
// Single-pid AX event subscription. Emits two event kinds to stdout:
//
//   ax.focused_window_changed  { pid, bundle_id, window_title }
//   ax.title_changed           { pid, bundle_id, window_title }
//
// Implementation note (divergence from capture-helper-windows-spec.md § 6.4):
// the spec calls for UIA event handlers driven from a dedicated STA pump.
// v1 uses Win32 SetWinEventHook (EVENT_SYSTEM_FOREGROUND +
// EVENT_OBJECT_NAMECHANGE) instead. It covers the contract for the cases
// that matter (cross-window focus + title rename) without the COM IUnknown
// boilerplate or the UIA-callback re-entry hazard. It does NOT catch
// pure intra-HWND subtree focus changes (e.g. Slack channel switch where
// the title stays identical) — that case is deferred to a v2 UIA-handler
// path.

#pragma once

#include <vector>

namespace corivo::ax {

/// Subscribe to events for `pid`. Replaces any prior subscription
/// atomically (one-pid-at-a-time, matching macOS AXObserver behaviour).
/// Starts the dedicated pump thread on first call.
void subscribe_pid(int pid);

/// Clear the current subscription. Pump thread keeps running for cheap
/// re-subscribe; it shuts down at process exit.
void unsubscribe_pid();

/// For heartbeat reporting. Returns the singleton list [pid] when a
/// subscription is active, empty vector otherwise.
std::vector<int> active_subscriptions();

/// Optional explicit shutdown — only used by tests. Production never
/// calls this; the pump thread terminates with the process.
void shutdown_event_bridge();

} // namespace corivo::ax
