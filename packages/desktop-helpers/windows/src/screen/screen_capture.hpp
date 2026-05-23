// screen_capture.hpp
// Single-frame screen capture via Windows.Graphics.Capture (WGC).
//
// Pipeline: WGC GraphicsCaptureItem (per-monitor) → Direct3D11CaptureFramePool
// → first FrameArrived → CPU staging texture → WIC encode → file on disk.

#pragma once

#include <chrono>
#include <optional>
#include <string>

namespace corivo::screen {

enum class ImageFormat { Jpeg, Png };

struct CaptureRequest {
    std::optional<std::string> display_id;     // empty → primary
    std::optional<std::string> output_path;    // empty → temp file
    ImageFormat format = ImageFormat::Jpeg;
    std::optional<int> quality;                // 0-100; ignored for PNG
};

struct CaptureResponse {
    std::string path;
    int width;
    int height;
    std::chrono::system_clock::time_point captured_at;
    std::string display_id;                    // resolved display id
};

/// Capture a single frame. Throws std::runtime_error on WGC / D3D / WIC
/// failures; the router maps that to a `OS_ERROR` response.
CaptureResponse capture(const CaptureRequest& request);

} // namespace corivo::screen
