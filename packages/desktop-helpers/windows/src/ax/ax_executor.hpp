// ax_executor.hpp
//
// Long-lived MTA thread pool for UIA calls. All ax.* handlers route their
// UIA-touching work through here instead of running it on the IPC
// dispatcher thread, for two reasons:
//
//   1. UIA has no per-call timeout (no equivalent of macOS's
//      AXUIElementSetMessagingTimeout). The handler races the task against
//      its own deadline via std::future::wait_for; on timeout the dispatch
//      thread returns TIMEOUT to the client while the worker is left to
//      finish (or never) — its result is then orphaned.
//   2. main.cpp detaches a fresh std::thread per request. Those threads
//      have no apartment initialized, so the first UIA call triggers an
//      implicit STA init. Routing through dedicated MTA workers eliminates
//      that per-request apartment churn.
//
// The pool is created lazily on first submit() and lives for the helper's
// lifetime. stop_pool() exists for tests; production never calls it.
//
// Submission policy: if the queue is already at QUEUE_CAP entries, submit()
// returns a future that is already in a failed state with code BUSY so the
// handler can immediately answer RESOURCE_BUSY instead of stacking work
// behind a wedged worker.

#pragma once

#include <chrono>
#include <functional>
#include <future>

namespace corivo::ax {

constexpr int  POOL_SIZE = 2;
constexpr int  QUEUE_CAP = 8;

enum class SubmitStatus { Accepted, Busy };

struct SubmitResult {
    SubmitStatus status = SubmitStatus::Accepted;
    std::future<void> future;
};

/// Submit a UIA-touching task to the long-lived MTA worker pool.
/// The task runs on a thread that has CoInitializeEx(MTA) already done.
/// Caller is expected to wait on `result.future` with their own deadline;
/// if the wait times out, the task is left running (result discarded by
/// the worker) and the caller must not touch any state the task wrote
/// after returning from wait_for.
SubmitResult submit(std::function<void()> task);

/// Idempotent. Production calls neither — pool starts on first submit()
/// and shuts down at process exit.
void start_pool();
void stop_pool();

} // namespace corivo::ax
