//
//  AccessibilityQuery.swift
//  Port of the Rust ax_extractor algorithm to Swift.
//
//  Walks the focused window of `pid` via the C-level AX API and stitches
//  role-tagged plaintext (`[TITLE]`, `[BUTTON]`, `[TAB]` markers inline,
//  everything else as plaintext). Hard caps on depth, total characters,
//  and wall-clock deadline so a pathological a11y tree (Slack / Discord)
//  can't blow the stack or starve the helper.
//

import ApplicationServices
import CoreGraphics
import Foundation

enum AccessibilityQuery {

    struct SkipPredicate {
        var skipRoles: Set<String> = []
        var skipSubroles: Set<String> = []
        var skipDescriptionsSubstr: [String] = []

        var isEmpty: Bool {
            skipRoles.isEmpty
                && skipSubroles.isEmpty
                && skipDescriptionsSubstr.isEmpty
        }
    }

    struct Request {
        var pid: pid_t
        var maxDepth: Int
        var maxChars: Int
        var deadlineSeconds: TimeInterval
        var skip: SkipPredicate
    }

    struct Response {
        var text: String
        var elapsedMs: UInt64
        var truncated: Bool
    }

    enum RoleTag: String {
        case title = "[TITLE]"
        case button = "[BUTTON]"
        case tab = "[TAB]"
    }

    static func query(_ request: Request) throws -> Response {
        guard AXIsProcessTrusted() else {
            throw HelperError.permissionDenied(
                "Accessibility permission missing for the helper bundle",
                kind: "accessibility"
            )
        }

        let app = AXUIElementCreateApplication(request.pid)
        // AXUIElementCreateApplication returns a +1 retained CFType; the
        // ARC bridge takes ownership and releases on scope exit.
        AXUIElementSetMessagingTimeout(app, Float(request.deadlineSeconds))

        guard let focused = copyAttribute(app, kAXFocusedWindowAttribute as CFString),
              CFGetTypeID(focused as CFTypeRef) == AXUIElementGetTypeID()
        else {
            throw HelperError.notFound("focused window unavailable for pid \(request.pid)")
        }
        let window = focused as! AXUIElement

        let started = Date()
        let deadline = started.addingTimeInterval(request.deadlineSeconds)
        var buffer = ""
        walk(
            element: window,
            depth: 0,
            buffer: &buffer,
            deadline: deadline,
            request: request
        )

        let elapsedMs = UInt64(Date().timeIntervalSince(started) * 1000)
        return Response(
            text: buffer,
            elapsedMs: elapsedMs,
            truncated: buffer.count >= request.maxChars
        )
    }

    private static func walk(
        element: AXUIElement,
        depth: Int,
        buffer: inout String,
        deadline: Date,
        request: Request
    ) {
        if depth > request.maxDepth { return }
        if Date() > deadline { return }
        if buffer.count >= request.maxChars { return }

        let role = stringValue(copyAttribute(element, kAXRoleAttribute as CFString))
        let subrole = stringValue(copyAttribute(element, "AXSubrole" as CFString))
        let description = stringValue(copyAttribute(element, "AXDescription" as CFString))

        if let r = role, request.skip.skipRoles.contains(r) { return }
        if let s = subrole, request.skip.skipSubroles.contains(s) { return }
        if let d = description {
            for needle in request.skip.skipDescriptionsSubstr {
                if d.range(of: needle) != nil { return }
            }
        }

        let tag = roleTag(role)

        if let title = stringValue(copyAttribute(element, kAXTitleAttribute as CFString)) {
            let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                appendTagged(into: &buffer, tag: tag, text: trimmed, maxChars: request.maxChars)
            }
        }

        if let value = stringValue(copyAttribute(element, kAXValueAttribute as CFString)) {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                appendTagged(into: &buffer, tag: nil, text: trimmed, maxChars: request.maxChars)
            }
        }

        guard let children = copyAttribute(element, kAXChildrenAttribute as CFString),
              CFGetTypeID(children as CFTypeRef) == CFArrayGetTypeID()
        else { return }

        let arr = children as! NSArray
        for item in arr {
            if Date() > deadline { return }
            if buffer.count >= request.maxChars { return }
            let child = item as! AXUIElement
            walk(
                element: child,
                depth: depth + 1,
                buffer: &buffer,
                deadline: deadline,
                request: request
            )
        }
    }

    private static func roleTag(_ role: String?) -> RoleTag? {
        guard let r = role else { return nil }
        if r == kAXButtonRole as String
            || r == kAXMenuButtonRole as String
            || r == kAXPopUpButtonRole as String
        {
            return .button
        }
        if r == kAXTabGroupRole as String { return .tab }
        if r == kAXWindowRole as String { return .title }
        return nil
    }

    private static func appendTagged(
        into buffer: inout String,
        tag: RoleTag?,
        text: String,
        maxChars: Int
    ) {
        if !buffer.isEmpty && !buffer.hasSuffix("\n") {
            buffer.append("\n")
        }
        if let tag = tag {
            buffer.append(tag.rawValue)
            buffer.append(" ")
        }
        let remaining = maxChars - buffer.count
        if remaining <= 0 { return }
        if text.count <= remaining {
            buffer.append(text)
        } else {
            buffer.append(String(text.prefix(remaining)))
        }
    }

    private static func copyAttribute(
        _ element: AXUIElement,
        _ attribute: CFString
    ) -> AnyObject? {
        var value: AnyObject?
        let err = AXUIElementCopyAttributeValue(element, attribute, &value)
        return err == .success ? value : nil
    }

    /// Coerce an AX-returned CFType into a String for plaintext stitching.
    /// Mirrors the Rust `cf_value_as_text` behaviour: strings pass through;
    /// numbers / booleans get a sensible textual rendering; anything else
    /// is dropped as not-useful-as-plaintext.
    private static func stringValue(_ value: AnyObject?) -> String? {
        guard let v = value else { return nil }
        if let s = v as? String { return s }
        if let n = v as? NSNumber {
            // Distinguish bool-shaped NSNumber from numeric.
            if CFGetTypeID(n) == CFBooleanGetTypeID() {
                return n.boolValue ? "true" : "false"
            }
            return n.stringValue
        }
        return nil
    }
}
