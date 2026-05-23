// iso8601.cpp

#include "iso8601.hpp"

#include <ctime>
#include <cstdio>

namespace corivo::iso8601 {

std::string now_utc() {
    return format(std::chrono::system_clock::now());
}

std::string format(std::chrono::system_clock::time_point tp) {
    auto since_epoch = tp.time_since_epoch();
    auto secs = std::chrono::duration_cast<std::chrono::seconds>(since_epoch);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                  since_epoch - secs)
                  .count();
    std::time_t t = static_cast<std::time_t>(secs.count());
    std::tm utc{};
    // gmtime_s on MSVC is the thread-safe variant. Returns 0 on success.
    if (gmtime_s(&utc, &t) != 0) {
        return "1970-01-01T00:00:00.000Z";
    }
    char date_part[24];
    std::strftime(date_part, sizeof(date_part), "%Y-%m-%dT%H:%M:%S", &utc);
    char out[40];
    std::snprintf(out, sizeof(out), "%s.%03lldZ", date_part,
                  static_cast<long long>(ms));
    return std::string(out);
}

} // namespace corivo::iso8601
