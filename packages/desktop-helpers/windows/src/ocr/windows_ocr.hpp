// windows_ocr.hpp
// Wrap Windows.Media.Ocr (WinRT) as the Phase 4 helper-side OCR. Pick the
// first language the OS has installed; if none of the requested languages
// are present we fall back to the user-profile-default engine.

#pragma once

#include <chrono>
#include <string>
#include <vector>

namespace corivo::ocr {

struct Request {
    std::string image_path;
    std::vector<std::string> languages;
    bool use_language_correction = false;
};

struct Response {
    std::string text;
    unsigned long long elapsed_ms = 0;
};

Response run(const Request& request);

} // namespace corivo::ocr
