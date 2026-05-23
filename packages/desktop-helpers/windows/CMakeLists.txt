# CorivoCaptureHelper (Windows)
# =============================
# Sidecar binary that the Rust capture_client spawns. Implements the
# cross-platform v1 protocol defined in:
#   apps/desktop/src-tauri/schemas/capture-helper-v1.json
#   apps/desktop/docs/capture-helper-architecture-spec.md
#   apps/desktop/docs/capture-helper-windows-spec.md
#
# Phase 0 scope (this file): control plane only — hello / heartbeat /
# ping / shutdown / `--echo-mode`. Audio (WASAPI/MF), screen (WGC),
# accessibility (UIA), foreground (SetWinEventHook), and OCR
# (Windows.Media.Ocr) come in their respective phases.

cmake_minimum_required(VERSION 3.20)
project(CorivoCaptureHelper LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# nlohmann/json via FetchContent so a fresh checkout doesn't require a
# pre-installed vcpkg. Pinned to a tagged release for reproducibility.
include(FetchContent)
FetchContent_Declare(
  nlohmann_json
  GIT_REPOSITORY https://github.com/nlohmann/json.git
  GIT_TAG        v3.11.3
)
FetchContent_MakeAvailable(nlohmann_json)

add_executable(CorivoCaptureHelper
  src/main.cpp
  src/ipc/codec.cpp
  src/ipc/event_emitter.cpp
  src/ipc/heartbeat.cpp
  src/ipc/router.cpp
  src/ipc/protocol.cpp
  src/control/control_handlers.cpp
  src/screen/screen_capture.cpp
  src/screen/display_enumerator.cpp
  src/screen/screen_handlers.cpp
  src/ax/accessibility_query.cpp
  src/ax/selection_probe.cpp
  src/ax/ax_executor.cpp
  src/ax/ax_handlers.cpp
  src/ax/ax_subscribe_handlers.cpp
  src/ax/ax_event_bridge.cpp
  src/foreground/foreground_monitor.cpp
  src/foreground/foreground_handlers.cpp
  src/ocr/windows_ocr.cpp
  src/ocr/ocr_handlers.cpp
  src/audio/audio_handlers.cpp
  src/permissions/permission_handlers.cpp
  src/util/logger.cpp
  src/util/iso8601.cpp
)

target_include_directories(CorivoCaptureHelper PRIVATE src)

target_link_libraries(CorivoCaptureHelper PRIVATE
  nlohmann_json::nlohmann_json
  # Phase 1 — WGC + D3D + WIC.
  windowsapp.lib            # cppwinrt projections (Windows.Graphics.Capture, ...)
  d3d11.lib
  dxgi.lib
  shcore.lib                # IGraphicsCaptureItemInterop's COM glue
  # Phase 2 — UI Automation.
  uiautomationcore.lib
  oleaut32.lib              # SysFreeString
  propsys.lib               # IPropertyStore (used by display enumerator + UIA cache requests)
)

if(MSVC)
  target_compile_options(CorivoCaptureHelper PRIVATE
    /W4
    /permissive-
    /utf-8
    /Zc:__cplusplus
  )
  target_compile_definitions(CorivoCaptureHelper PRIVATE
    NOMINMAX
    WIN32_LEAN_AND_MEAN
    _WIN32_WINNT=0x0A00
    UNICODE
    _UNICODE
    # Silence MSVC's "unsafe CRT function" nudges (C4996) for the C
    # standard-library calls we use deliberately — std::getenv in
    # heartbeat.cpp etc. The "_s" replacements MSVC suggests are
    # Microsoft-only and would break source portability with the macOS
    # helper. Scoped to this target so it doesn't leak into the rest of
    # the workspace.
    _CRT_SECURE_NO_WARNINGS
  )
  set_target_properties(CorivoCaptureHelper PROPERTIES
    LINK_FLAGS "/SUBSYSTEM:CONSOLE"
  )
endif()

# When phases 2-5 land, append:
#   mmdevapi.lib audioses.lib            # WASAPI (audio capture)
#   mfplat.lib mfreadwrite.lib mfuuid.lib mf.lib   # Media Foundation
#   uiautomationcore.lib propsys.lib     # UIA + property store
#   avrt.lib                             # AvSetMmThreadCharacteristics
