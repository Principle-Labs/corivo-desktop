// codec.cpp

#include "codec.hpp"

#include <algorithm>
#include <vector>

namespace corivo::ipc {

LineReader::LineReader(HANDLE handle) : handle_(handle) {}

std::optional<std::string> LineReader::read_line() {
    while (true) {
        auto pos = std::find(buffer_.begin(), buffer_.end(), '\n');
        if (pos != buffer_.end()) {
            std::string line(buffer_.begin(), pos);
            // Strip trailing \r if present (Tauri pipes are byte-mode but
            // a misconfigured shell-level wrapper might still produce CRLF).
            if (!line.empty() && line.back() == '\r') {
                line.pop_back();
            }
            buffer_.erase(buffer_.begin(), pos + 1);
            return line;
        }

        // Need more bytes.
        char chunk[4096];
        DWORD n = 0;
        BOOL ok = ReadFile(handle_, chunk, sizeof(chunk), &n, nullptr);
        if (!ok) {
            DWORD err = GetLastError();
            // ERROR_BROKEN_PIPE happens when the parent (writer) closes
            // their end. ERROR_HANDLE_EOF is the formal EOF return.
            if (err == ERROR_BROKEN_PIPE || err == ERROR_HANDLE_EOF) {
                if (buffer_.empty()) return std::nullopt;
                std::string last(buffer_);
                buffer_.clear();
                return last;
            }
            throw CodecError("ReadFile failed: " + std::to_string(err));
        }
        if (n == 0) {
            if (buffer_.empty()) return std::nullopt;
            std::string last(buffer_);
            buffer_.clear();
            return last;
        }
        buffer_.append(chunk, n);
        if (buffer_.size() > MAX_LINE_BYTES) {
            throw CodecError("line exceeded " +
                             std::to_string(MAX_LINE_BYTES) +
                             " bytes without newline");
        }
    }
}

} // namespace corivo::ipc
