// accessibility_query.hpp
// UIA-based focused-window text walk. Mirrors the Phase 2 macOS Swift
// `AccessibilityQuery` shape so the wire response is identical.

#pragma once

#include <chrono>
#include <optional>
#include <set>
#include <string>
#include <vector>

namespace corivo::ax {

struct SkipPredicate {
    std::set<std::string> skip_roles;            // matched against UIA control type names
    std::set<std::string> skip_subroles;         // (UIA has no subrole; reserved for parity)
    std::vector<std::string> skip_descriptions_substr;

    bool empty() const {
        return skip_roles.empty()
            && skip_subroles.empty()
            && skip_descriptions_substr.empty();
    }
};

struct QueryRequest {
    int pid = 0;
    int max_depth = 64;
    int max_chars = 64000;
    std::chrono::milliseconds deadline{1500};
    SkipPredicate skip;
};

struct QueryResponse {
    std::string text;
    unsigned long long elapsed_ms = 0;
    bool truncated = false;

    /// Non-error explanation for why text is empty (or partial). Lets the
    /// Rust side decide whether to fall back to OCR without parsing
    /// free-form error messages. Empty/null when the walk succeeded
    /// normally. Values used today:
    ///   - "no_focused_window"  — no window for `pid` was visible to us
    ///   - "elevated_target"    — pid is owned by a higher-integrity
    ///                            process (helper runs `asInvoker`)
    ///   - "empty_tree"         — window was found but yielded no text
    std::optional<std::string> reason;
};

QueryResponse query(const QueryRequest& request);

} // namespace corivo::ax
