// iso8601.hpp
// Format DateTime values into the same RFC3339-with-millis-Z form that the
// Rust client (chrono::DateTime<Utc>) emits and parses, so heartbeats and
// event timestamps round-trip cleanly.

#pragma once

#include <chrono>
#include <string>

namespace corivo::iso8601 {

/// Format `now()` as e.g. "2026-05-06T12:34:56.789Z".
std::string now_utc();

/// Format an arbitrary `system_clock::time_point` as RFC3339-with-millis-Z.
std::string format(std::chrono::system_clock::time_point tp);

} // namespace corivo::iso8601
