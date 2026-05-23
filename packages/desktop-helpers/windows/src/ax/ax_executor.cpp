// ax_executor.cpp

#include "ax_executor.hpp"

#include <condition_variable>
#include <deque>
#include <mutex>
#include <thread>
#include <vector>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>
#include <objbase.h>

#include "../util/logger.hpp"

namespace corivo::ax {

namespace {

class Pool {
public:
    void start() {
        std::lock_guard<std::mutex> lk(mu_);
        if (running_) return;
        running_ = true;
        for (int i = 0; i < POOL_SIZE; ++i) {
            workers_.emplace_back([this, i] { worker_loop(i); });
        }
    }

    void stop() {
        {
            std::lock_guard<std::mutex> lk(mu_);
            running_ = false;
        }
        cv_.notify_all();
        for (auto& t : workers_) {
            if (t.joinable()) t.join();
        }
        workers_.clear();
    }

    SubmitResult submit(std::function<void()> task) {
        start();  // lazy

        std::packaged_task<void()> pkg(std::move(task));
        auto fut = pkg.get_future();

        {
            std::lock_guard<std::mutex> lk(mu_);
            if (static_cast<int>(queue_.size()) >= QUEUE_CAP) {
                // Refuse — set the promise to a generic exception so the
                // caller's wait_for() returns ready immediately. We use a
                // sentinel exception type encoded in the message so the
                // handler can map it to RESOURCE_BUSY.
                std::packaged_task<void()> rejected([]{
                    throw std::runtime_error("ax_executor: queue full");
                });
                auto rej_fut = rejected.get_future();
                rejected();  // execute synchronously so future is ready
                return SubmitResult{SubmitStatus::Busy, std::move(rej_fut)};
            }
            queue_.emplace_back(std::move(pkg));
        }
        cv_.notify_one();
        return SubmitResult{SubmitStatus::Accepted, std::move(fut)};
    }

private:
    void worker_loop(int idx) {
        HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        if (FAILED(hr)) {
            log::error("ax_executor: CoInitializeEx(MTA) failed on worker "
                       + std::to_string(idx)
                       + " hr=0x" + std::to_string(static_cast<unsigned long>(hr)));
            return;
        }

        for (;;) {
            std::packaged_task<void()> task;
            {
                std::unique_lock<std::mutex> lk(mu_);
                cv_.wait(lk, [this]{ return !running_ || !queue_.empty(); });
                if (!running_ && queue_.empty()) break;
                task = std::move(queue_.front());
                queue_.pop_front();
            }
            try {
                task();
            } catch (...) {
                // packaged_task captures exceptions into the future; this
                // catch is only reached if task() itself somehow escapes
                // packaged_task's bookkeeping, which std doesn't allow.
            }
        }

        CoUninitialize();
    }

    std::mutex mu_;
    std::condition_variable cv_;
    std::deque<std::packaged_task<void()>> queue_;
    std::vector<std::thread> workers_;
    bool running_ = false;
};

Pool& pool() {
    static Pool instance;
    return instance;
}

} // namespace

SubmitResult submit(std::function<void()> task) {
    return pool().submit(std::move(task));
}

void start_pool() { pool().start(); }
void stop_pool()  { pool().stop(); }

} // namespace corivo::ax
