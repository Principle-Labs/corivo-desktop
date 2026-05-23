// logger.hpp
// Tiny stderr logger. The Rust capture_client forwards each line into its
// `tracing` pipeline (target `capture_helper`).

#pragma once

#include <string>
#include <string_view>

namespace corivo::log {

enum class Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
};

void write(Level level, std::string_view message);

inline void trace(std::string_view m) { write(Level::Trace, m); }
inline void debug(std::string_view m) { write(Level::Debug, m); }
inline void info(std::string_view m)  { write(Level::Info,  m); }
inline void warn(std::string_view m)  { write(Level::Warn,  m); }
inline void error(std::string_view m) { write(Level::Error, m); }

} // namespace corivo::log
