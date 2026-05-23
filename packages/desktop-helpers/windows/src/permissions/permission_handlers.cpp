// permission_handlers.cpp

#include "permission_handlers.hpp"

#include <winrt/base.h>
#include <winrt/Windows.Devices.Enumeration.h>

#include "../ipc/event_emitter.hpp"
#include "../util/logger.hpp"

namespace corivo::permissions {

namespace {

const char* microphone_state() {
    using namespace winrt::Windows::Devices::Enumeration;
    try {
        auto info = DeviceAccessInformation::CreateFromDeviceClass(
            DeviceClass::AudioCapture);
        switch (info.CurrentStatus()) {
            case DeviceAccessStatus::Allowed:        return "granted";
            case DeviceAccessStatus::DeniedBySystem:
            case DeviceAccessStatus::DeniedByUser:   return "denied";
            case DeviceAccessStatus::Unspecified:    return "undetermined";
            default:                                 return "undetermined";
        }
    } catch (const winrt::hresult_error&) {
        // CreateFromDeviceClass can throw on un-packaged Win32 in some
        // sandbox configurations; treat as undetermined.
        return "undetermined";
    }
}

} // namespace

void handle_status(const corivo::proto::Request& request) {
    nlohmann::json result = {
        {"microphone", microphone_state()},
        {"screen_recording", "granted"},  // WGC needs no per-process permission
        {"accessibility", "granted"},     // UIA needs no per-process permission
    };
    corivo::proto::Response resp;
    resp.id = request.id;
    resp.ok = true;
    resp.result = result;
    corivo::ipc::EventEmitter::instance().send(resp);
}

} // namespace corivo::permissions
