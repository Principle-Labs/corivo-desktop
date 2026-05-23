// codec.hpp
// NDJSON line reader. Reads from stdin (the pipe Tauri opens for us).
// Hard line-length cap protects us from a misbehaving client.

#pragma once

#include <cstddef>
#include <optional>
#include <stdexcept>
#include <string>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

namespace corivo::ipc {

inline constexpr std::size_t MAX_LINE_BYTES = 1024 * 1024;

class CodecError : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

/// Single-reader line stream over a HANDLE (typically GetStdHandle(STD_INPUT_HANDLE)).
class LineReader {
public:
    explicit LineReader(HANDLE handle);

    /// Returns the next line (without trailing \r\n). Returns nullopt on
    /// EOF / broken-pipe. Throws [`CodecError`] on lines exceeding
    /// [`MAX_LINE_BYTES`].
    std::optional<std::string> read_line();

private:
    HANDLE handle_;
    std::string buffer_;
};

} // namespace corivo::ipc
