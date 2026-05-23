// accessibility_query.cpp
//
// UIA tree walk for the focused window of `pid`. Steps:
//   1. Build an AND condition (ProcessId == pid AND ControlType == Window).
//   2. Build an IUIAutomationCacheRequest that pre-fetches every property
//      and pattern the walk reads — ControlType, Name, HelpText, and the
//      Value pattern. Without this each visited node costs one cross-
//      process RPC per property; with it the whole node lands in one round
//      trip. Chromium / Electron a11y trees have hundreds of nodes, so
//      this is the difference between a 700ms query (spec § 14 budget)
//      and a multi-second one.
//   3. Use FindFirstBuildCache from the desktop root to land on the
//      target window with the cache populated.
//   4. Walk via ControlViewWalker's BuildCache-aware methods, reading
//      properties exclusively via GetCachedPropertyValue / GetCachedPattern.
//   5. Stitch role-tagged plaintext mirroring the macOS marker format.
//
// UIA has no per-call timeout equivalent of AXUIElementSetMessagingTimeout,
// so we rely on the inner deadline check + tree pruning to bound wall-
// clock. The handler (ax_handlers.cpp) additionally races us against an
// outer future deadline via the AXExecutor pool.

#include "accessibility_query.hpp"

#include <chrono>
#include <stdexcept>
#include <string>

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

void check(HRESULT hr, const char* where) {
    if (FAILED(hr)) {
        char buf[160];
        std::snprintf(buf, sizeof(buf), "%s failed: HRESULT 0x%08lX", where,
                      static_cast<unsigned long>(hr));
        throw std::runtime_error(buf);
    }
}

ComPtr<IUIAutomation> get_uia() {
    thread_local ComPtr<IUIAutomation> instance;
    if (!instance) {
        check(CoCreateInstance(CLSID_CUIAutomation8, nullptr,
                               CLSCTX_INPROC_SERVER,
                               IID_PPV_ARGS(instance.GetAddressOf())),
              "CoCreateInstance(IUIAutomation)");
    }
    return instance;
}

std::string bstr_to_utf8(BSTR b) {
    if (!b) return {};
    int wlen = static_cast<int>(SysStringLen(b));
    if (wlen == 0) return {};
    int len = WideCharToMultiByte(CP_UTF8, 0, b, wlen, nullptr, 0,
                                   nullptr, nullptr);
    std::string out;
    out.resize(len);
    WideCharToMultiByte(CP_UTF8, 0, b, wlen, out.data(), len, nullptr, nullptr);
    return out;
}

const char* role_tag_for(CONTROLTYPEID type) {
    switch (type) {
        case UIA_WindowControlTypeId:    return "[TITLE]";
        case UIA_ButtonControlTypeId:
        case UIA_MenuItemControlTypeId:  return "[BUTTON]";
        case UIA_TabControlTypeId:       return "[TAB]";
        default:                         return nullptr;
    }
}

const char* control_type_name(CONTROLTYPEID type) {
    switch (type) {
        case UIA_ButtonControlTypeId:        return "Button";
        case UIA_CalendarControlTypeId:      return "Calendar";
        case UIA_CheckBoxControlTypeId:      return "CheckBox";
        case UIA_ComboBoxControlTypeId:      return "ComboBox";
        case UIA_EditControlTypeId:          return "Edit";
        case UIA_HyperlinkControlTypeId:     return "Hyperlink";
        case UIA_ImageControlTypeId:         return "Image";
        case UIA_ListItemControlTypeId:      return "ListItem";
        case UIA_ListControlTypeId:          return "List";
        case UIA_MenuControlTypeId:          return "Menu";
        case UIA_MenuBarControlTypeId:       return "MenuBar";
        case UIA_MenuItemControlTypeId:      return "MenuItem";
        case UIA_ProgressBarControlTypeId:   return "ProgressBar";
        case UIA_RadioButtonControlTypeId:   return "RadioButton";
        case UIA_ScrollBarControlTypeId:     return "ScrollBar";
        case UIA_SliderControlTypeId:        return "Slider";
        case UIA_SpinnerControlTypeId:       return "Spinner";
        case UIA_StatusBarControlTypeId:     return "StatusBar";
        case UIA_TabControlTypeId:           return "Tab";
        case UIA_TabItemControlTypeId:       return "TabItem";
        case UIA_TextControlTypeId:          return "Text";
        case UIA_ToolBarControlTypeId:       return "ToolBar";
        case UIA_ToolTipControlTypeId:       return "ToolTip";
        case UIA_TreeControlTypeId:          return "Tree";
        case UIA_TreeItemControlTypeId:      return "TreeItem";
        case UIA_CustomControlTypeId:        return "Custom";
        case UIA_GroupControlTypeId:         return "Group";
        case UIA_ThumbControlTypeId:         return "Thumb";
        case UIA_DataGridControlTypeId:      return "DataGrid";
        case UIA_DataItemControlTypeId:      return "DataItem";
        case UIA_DocumentControlTypeId:      return "Document";
        case UIA_SplitButtonControlTypeId:   return "SplitButton";
        case UIA_WindowControlTypeId:        return "Window";
        case UIA_PaneControlTypeId:          return "Pane";
        case UIA_HeaderControlTypeId:        return "Header";
        case UIA_HeaderItemControlTypeId:    return "HeaderItem";
        case UIA_TableControlTypeId:         return "Table";
        case UIA_TitleBarControlTypeId:      return "TitleBar";
        case UIA_SeparatorControlTypeId:     return "Separator";
        default:                             return "Unknown";
    }
}

void append_tagged(std::string& buffer, const char* tag, const std::string& text,
                   int max_chars) {
    if (!buffer.empty() && buffer.back() != '\n') buffer.push_back('\n');
    if (tag) {
        buffer.append(tag);
        buffer.push_back(' ');
    }
    int remaining = max_chars - static_cast<int>(buffer.size());
    if (remaining <= 0) return;
    if (static_cast<int>(text.size()) <= remaining) {
        buffer.append(text);
    } else {
        buffer.append(text.data(), remaining);
    }
}

// One cache request is reused for the whole walk — UIA's BuildCache
// variants attach this request to every visited node, so each cross-
// process round trip returns ControlType + Name + HelpText + cached
// ValuePattern together.
ComPtr<IUIAutomationCacheRequest> build_cache_request(IUIAutomation* uia) {
    ComPtr<IUIAutomationCacheRequest> req;
    check(uia->CreateCacheRequest(req.GetAddressOf()), "CreateCacheRequest");
    check(req->AddProperty(UIA_ControlTypePropertyId),
          "AddProperty(ControlType)");
    check(req->AddProperty(UIA_NamePropertyId),
          "AddProperty(Name)");
    check(req->AddProperty(UIA_HelpTextPropertyId),
          "AddProperty(HelpText)");
    check(req->AddPattern(UIA_ValuePatternId),
          "AddPattern(Value)");
    req->put_TreeScope(TreeScope_Element);
    req->put_AutomationElementMode(AutomationElementMode_Full);
    return req;
}

CONTROLTYPEID cached_control_type(IUIAutomationElement* el) {
    VARIANT v; VariantInit(&v);
    CONTROLTYPEID out = 0;
    if (SUCCEEDED(el->GetCachedPropertyValue(UIA_ControlTypePropertyId, &v))
        && v.vt == VT_I4) {
        out = v.lVal;
    }
    VariantClear(&v);
    return out;
}

std::string cached_string_property(IUIAutomationElement* el, PROPERTYID prop) {
    VARIANT v; VariantInit(&v);
    HRESULT hr = el->GetCachedPropertyValue(prop, &v);
    std::string out;
    if (SUCCEEDED(hr) && v.vt == VT_BSTR) {
        out = bstr_to_utf8(v.bstrVal);
    }
    VariantClear(&v);
    return out;
}

std::string cached_value_pattern_value(IUIAutomationElement* el) {
    ComPtr<IUIAutomationValuePattern> vp;
    HRESULT hr = el->GetCachedPatternAs(UIA_ValuePatternId,
                                         IID_PPV_ARGS(vp.GetAddressOf()));
    if (FAILED(hr) || !vp) return {};
    BSTR val = nullptr;
    if (FAILED(vp->get_CachedValue(&val)) || !val) return {};
    std::string out = bstr_to_utf8(val);
    SysFreeString(val);
    return out;
}

bool deadline_hit(std::chrono::steady_clock::time_point start,
                  std::chrono::milliseconds budget) {
    return std::chrono::steady_clock::now() - start > budget;
}

void walk(IUIAutomationElement* element,
          int depth,
          std::string& buffer,
          std::chrono::steady_clock::time_point start,
          const QueryRequest& req,
          IUIAutomationTreeWalker* walker,
          IUIAutomationCacheRequest* cache_req) {
    if (depth > req.max_depth) return;
    if (deadline_hit(start, req.deadline)) return;
    if (static_cast<int>(buffer.size()) >= req.max_chars) return;

    CONTROLTYPEID type = cached_control_type(element);

    std::string role_name = control_type_name(type);
    if (req.skip.skip_roles.find(role_name) != req.skip.skip_roles.end()) return;

    std::string description = cached_string_property(element, UIA_HelpTextPropertyId);
    if (!description.empty()) {
        for (auto const& needle : req.skip.skip_descriptions_substr) {
            if (description.find(needle) != std::string::npos) return;
        }
    }

    const char* tag = role_tag_for(type);

    std::string name = cached_string_property(element, UIA_NamePropertyId);
    if (!name.empty()) {
        append_tagged(buffer, tag, name, req.max_chars);
    }

    std::string value = cached_value_pattern_value(element);
    if (!value.empty()) {
        append_tagged(buffer, nullptr, value, req.max_chars);
    }

    ComPtr<IUIAutomationElement> child;
    HRESULT hr = walker->GetFirstChildElementBuildCache(
        element, cache_req, child.GetAddressOf());
    while (SUCCEEDED(hr) && child) {
        if (deadline_hit(start, req.deadline)) return;
        if (static_cast<int>(buffer.size()) >= req.max_chars) return;
        walk(child.Get(), depth + 1, buffer, start, req, walker, cache_req);
        ComPtr<IUIAutomationElement> next;
        hr = walker->GetNextSiblingElementBuildCache(
            child.Get(), cache_req, next.GetAddressOf());
        child = next;
    }
}

} // namespace

QueryResponse query(const QueryRequest& req) {
    auto uia = get_uia();
    auto cache_req = build_cache_request(uia.Get());

    VARIANT v_pid; VariantInit(&v_pid); v_pid.vt = VT_I4; v_pid.lVal = req.pid;
    ComPtr<IUIAutomationCondition> pid_cond;
    check(uia->CreatePropertyCondition(UIA_ProcessIdPropertyId, v_pid,
                                        pid_cond.GetAddressOf()),
          "CreatePropertyCondition(pid)");

    VARIANT v_type; VariantInit(&v_type); v_type.vt = VT_I4;
    v_type.lVal = UIA_WindowControlTypeId;
    ComPtr<IUIAutomationCondition> window_cond;
    check(uia->CreatePropertyCondition(UIA_ControlTypePropertyId, v_type,
                                        window_cond.GetAddressOf()),
          "CreatePropertyCondition(window)");

    ComPtr<IUIAutomationCondition> and_cond;
    check(uia->CreateAndCondition(pid_cond.Get(), window_cond.Get(),
                                   and_cond.GetAddressOf()),
          "CreateAndCondition");

    ComPtr<IUIAutomationElement> root;
    check(uia->GetRootElement(root.GetAddressOf()), "GetRootElement");

    ComPtr<IUIAutomationElement> window;
    check(root->FindFirstBuildCache(TreeScope_Children, and_cond.Get(),
                                     cache_req.Get(), window.GetAddressOf()),
          "FindFirstBuildCache(window for pid)");
    if (!window) {
        // Fallback: look at the focused element and re-fetch with the cache
        // populated. If BuildUpdatedCache fails we treat this as a miss
        // rather than walking with uncached accessors, because the walk
        // reads every property via GetCached*.
        ComPtr<IUIAutomationElement> focused;
        if (SUCCEEDED(uia->GetFocusedElement(focused.GetAddressOf())) && focused) {
            int focused_pid = 0;
            focused->get_CurrentProcessId(&focused_pid);
            if (focused_pid == req.pid) {
                ComPtr<IUIAutomationElement> cached;
                if (SUCCEEDED(focused->BuildUpdatedCache(cache_req.Get(),
                                                         cached.GetAddressOf()))
                    && cached) {
                    window = cached;
                }
            }
        }
    }
    auto start = std::chrono::steady_clock::now();
    QueryResponse out;

    if (!window) {
        // Not an error from the protocol's perspective: it's a fact the
        // caller can act on (typically by falling back to OCR). Probe
        // whether the pid is even visible to us at PROCESS_QUERY_INFORMATION
        // — that access is what fails first when the target runs at a
        // higher integrity level than the helper. ERROR_ACCESS_DENIED →
        // elevated_target; everything else → no_focused_window.
        HANDLE probe = OpenProcess(PROCESS_QUERY_INFORMATION, FALSE,
                                    static_cast<DWORD>(req.pid));
        if (probe == nullptr && GetLastError() == ERROR_ACCESS_DENIED) {
            out.reason = std::string("elevated_target");
        } else {
            if (probe) CloseHandle(probe);
            out.reason = std::string("no_focused_window");
        }
        out.elapsed_ms = static_cast<unsigned long long>(
            std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now() - start).count());
        return out;
    }

    ComPtr<IUIAutomationTreeWalker> walker;
    check(uia->get_ControlViewWalker(walker.GetAddressOf()),
          "get_ControlViewWalker");

    out.text.reserve(static_cast<size_t>(req.max_chars));
    walk(window.Get(), 0, out.text, start, req, walker.Get(), cache_req.Get());
    out.elapsed_ms = static_cast<unsigned long long>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now() - start).count());
    out.truncated = static_cast<int>(out.text.size()) >= req.max_chars;
    if (out.text.empty()) {
        out.reason = std::string("empty_tree");
    }
    return out;
}

} // namespace corivo::ax
