// logger.cpp

#include "logger.hpp"

#include <cstdio>
#include <mutex>

namespace corivo::log {

namespace {

constexpr const char* level_label(Level l) {
    switch (l) {
        case Level::Trace: return "trace";
        case Level::Debug: return "debug";
        case Level::Info:  return "info";
        case Level::Warn:  return "warn";
        case Level::Error: return "error";
    }
    return "info";
}

std::mutex& mutex() {
    static std::mutex m;
    return m;
}

} // namespace

void write(Level level, std::string_view message) {
    // Stderr writes are line-atomic on Windows console / pipe by virtue of
    // a single fputs+fflush under our mutex. The Rust side reads stderr
    // line-by-line so we never split a log entry.
    std::lock_guard<std::mutex> lk(mutex());
    std::fprintf(stderr, "[%s] %.*s\n",
                 level_label(level),
                 static_cast<int>(message.size()),
                 message.data());
    std::fflush(stderr);
}

} // namespace corivo::log
