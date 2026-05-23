// selection_probe.cpp

#include "selection_probe.hpp"

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>

#include <wrl/client.h>
#include <UIAutomation.h>
#include <oleauto.h>

namespace corivo::ax {

namespace {

using Microsoft::WRL::ComPtr;

ComPtr<IUIAutomation> get_uia() {
    thread_local ComPtr<IUIAutomation> instance;
    if (!instance) {
        if (FAILED(CoCreateInstance(CLSID_CUIAutomation8, nullptr,
                                    CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(instance.GetAddressOf())))) {
            instance.Reset();
        }
    }
    return instance;
}

std::string bstr_to_utf8(BSTR b) {
    if (!b) return {};
    int wlen = static_cast<int>(SysStringLen(b));
    if (wlen == 0) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, b, wlen, nullptr, 0, nullptr, nullptr);
    std::string out;
    out.resize(len);
    WideCharToMultiByte(CP_UTF8, 0, b, wlen, out.data(), len, nullptr, nullptr);
    return out;
}

} // namespace

std::optional<std::string> probe_selection(int pid,
                                            std::chrono::milliseconds deadline) {
    auto uia = get_uia();
    if (!uia) return std::nullopt;

    auto start = std::chrono::steady_clock::now();
    auto deadline_hit = [&]() {
        return std::chrono::steady_clock::now() - start > deadline;
    };

    ComPtr<IUIAutomationElement> focused;
    if (FAILED(uia->GetFocusedElement(focused.GetAddressOf())) || !focused) {
        return std::nullopt;
    }
    if (deadline_hit()) return std::nullopt;

    int focused_pid = 0;
    if (FAILED(focused->get_CurrentProcessId(&focused_pid))) return std::nullopt;
    if (focused_pid != pid) return std::nullopt;

    ComPtr<IUIAutomationTextPattern> tp;
    if (FAILED(focused->GetCurrentPatternAs(UIA_TextPatternId,
                                             IID_PPV_ARGS(tp.GetAddressOf())))
        || !tp) {
        return std::nullopt;
    }

    ComPtr<IUIAutomationTextRangeArray> ranges;
    if (FAILED(tp->GetSelection(ranges.GetAddressOf())) || !ranges) {
        return std::nullopt;
    }

    int count = 0;
    ranges->get_Length(&count);
    if (count == 0) return std::nullopt;

    std::string out;
    for (int i = 0; i < count; ++i) {
        if (deadline_hit()) break;
        ComPtr<IUIAutomationTextRange> range;
        if (FAILED(ranges->GetElement(i, range.GetAddressOf())) || !range) continue;
        BSTR text = nullptr;
        if (FAILED(range->GetText(/* maxLength */ -1, &text)) || !text) continue;
        std::string chunk = bstr_to_utf8(text);
        SysFreeString(text);
        if (chunk.empty()) continue;
        if (!out.empty()) out.push_back('\n');
        out.append(chunk);
    }

    if (out.empty()) return std::nullopt;
    // Trim — return nullopt if it's all whitespace, matching macOS behaviour.
    // `find_last_not_of` is guaranteed to return a valid index whenever
    // `first` did (same predicate), so we don't need to re-check `last`.
    auto first = out.find_first_not_of(" \t\r\n");
    if (first == std::string::npos) return std::nullopt;
    auto last = out.find_last_not_of(" \t\r\n");
    return out.substr(first, last - first + 1);
}

} // namespace corivo::ax
